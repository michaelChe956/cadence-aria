//! 实体 scope 操作准入原语（fix-cascade-deletion Task D1.0，plan §0.2/§0.4/§0.5；
//! REQ-DEL-06/07）。
//!
//! 锁序（plan §0.2 固定序）：scope 集合（含引用上游，外层排序）→ 稳定 repo
//! resource 锁 → issue 关系短锁 → session/object 锁。**持 session/engine Mutex
//! 或任何对象 flock 期间禁止首次取 scope 锁**；内层只消费已持 lease 的
//! [`EntityRunLease::with_admitted_write`]，不重取 scope。
//!
//! - scope 锁：try-shared（运行准入/迟到写防御）与 try-exclusive（删除准入）
//!   均非阻塞，锁争用立即返回稳定错误，绝不等待 scope；
//! - 锁文件位于 `{issue root}/entity-operations/` 稳定元数据目录，inode 不随
//!   实体子树 purge 删除；
//! - run 不持关系锁；delete 只 try 独占；repo resource 写者/清理均为非阻塞
//!   独占短 lease 且互相互斥；
//! - 会话身份索引（state_root 下 `entity-identity/`）与全项目会话高水位是
//!   纯 ID/scope 准入事实，不存正文/hash；已删实体的 session/实体 ID 不复用。

use std::collections::BTreeSet;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::locking::{StableFileLock, with_exact_exclusive_lock};
use crate::product::entity_deletion_store::{
    DeletionPhase, EntityDeletionRecord, EntityScope,
    entity_deletion_record_path, issue_entity_operations_root, read_json_if_exists,
};
use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, write_json};
use crate::product::logical_codebase::LogicalRepositoryId;

/// scope 准入错误（plan §0.2）。
#[derive(Debug, thiserror::Error)]
pub enum EntityOperationError {
    /// 同 scope 仍有可落盘 writer（运行共享准入持有中）→ 稳定 `entity_busy`。
    #[error("entity_operation_busy")]
    Busy,
    /// 删除独占进行中（purging 墓碑或独占锁争用）→ 稳定 `entity_deleting`。
    #[error("entity_operation_deleting")]
    Deleting,
    /// 删除已完成（deleted 墓碑）→ 稳定拒绝，防重复清理/旧身份复活。
    #[error("entity_operation_deleted")]
    Deleted,
    /// 底层存储错误。
    #[error("entity_operation_store: {0}")]
    Store(#[from] ProductStoreError),
}

/// 稳定 repo resource 键（plan §0.2）：`(project, issue, repository)`；
/// `repository_id=None` 仅真实 Legacy issue shared 路径。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SharedResourceKey {
    pub project_id: String,
    pub issue_id: String,
    pub repository_id: Option<LogicalRepositoryId>,
}

/// 共享运行 lease：最外层取得，任务全生命周期持有；内层写消费
/// [`EntityRunLease::with_admitted_write`]，不重取 scope。
pub struct EntityRunLease {
    _lock: StableFileLock,
    scope: EntityScope,
    record_path: PathBuf,
}

impl EntityRunLease {
    pub fn scope(&self) -> &EntityScope {
        &self.scope
    }

    /// 已准入写（plan §0.4）：在既有 session/object 锁内消费外层已持 run
    /// lease——只复核墓碑，不重取 scope 锁（嵌套首次获取被禁止）。
    pub fn with_admitted_write<T>(
        &self,
        operation: impl FnOnce() -> Result<T, ProductStoreError>,
    ) -> Result<T, ProductStoreError> {
        match read_json_if_exists::<EntityDeletionRecord>(&self.record_path)? {
            None => operation(),
            Some(_) => Err(ProductStoreError::Conflict {
                kind: "entity_deleting",
                id: self.scope.entity_id.clone(),
            }),
        }
    }
}

/// 删除独占 lease（try-exclusive）：purging 全程持有，是 `begin`/`complete` 与
/// pending/owned-index 写入的唯一合法凭证。
pub struct EntityDeletionLease {
    _lock: StableFileLock,
    scope: EntityScope,
}

impl EntityDeletionLease {
    pub fn scope(&self) -> &EntityScope {
        &self.scope
    }
}

/// repo resource 写者短 lease（非阻塞独占）。
pub struct SharedResourceWriteLease {
    _lock: StableFileLock,
    key: SharedResourceKey,
}

impl SharedResourceWriteLease {
    pub fn key(&self) -> &SharedResourceKey {
        &self.key
    }
}

/// repo resource 清理短 lease（非阻塞独占，与写者互斥）。
pub struct SharedResourceCleanupLease {
    _lock: StableFileLock,
    key: SharedResourceKey,
}

impl SharedResourceCleanupLease {
    pub fn key(&self) -> &SharedResourceKey {
        &self.key
    }
}

/// 实体 scope 操作准入存储（plan §0.2/§0.5，D1.0）。
#[derive(Debug, Clone)]
pub struct EntityOperationStore {
    paths: ProductAppPaths,
}

impl EntityOperationStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    /// 共享运行准入：try-shared 非阻塞；争用按墓碑分类（purging→Deleting、
    /// deleted→Deleted、无墓碑→Busy）；取得后锁内复核墓碑。
    pub fn acquire_run(&self, scope: &EntityScope) -> Result<EntityRunLease, EntityOperationError> {
        let record_path = entity_deletion_record_path(&self.paths, scope)?;
        let lock_path = scope_lock_path(&self.paths, scope)?;
        let Some(lock) = StableFileLock::try_acquire_shared(&lock_path)? else {
            // 争用：删除独占进行中（或并发准入）；按墓碑给出稳定分类。
            return Err(self.scope_contention_error(&record_path));
        };
        // 锁内复核墓碑：run 准入不得落在 purging/deleted 身份上。
        match read_json_if_exists::<EntityDeletionRecord>(&record_path)? {
            None => Ok(EntityRunLease {
                _lock: lock,
                scope: scope.clone(),
                record_path,
            }),
            Some(record) => Err(record.phase.into()),
        }
    }

    /// 删除准入：try-exclusive 非阻塞；run 持共享→Busy；锁内复核墓碑
    /// （deleted 稳定拒绝；purging 允许删除重入）。
    pub fn try_acquire_delete(
        &self,
        scope: &EntityScope,
    ) -> Result<EntityDeletionLease, EntityOperationError> {
        let record_path = entity_deletion_record_path(&self.paths, scope)?;
        let lock_path = scope_lock_path(&self.paths, scope)?;
        let Some(lock) = StableFileLock::try_acquire_exclusive(&lock_path)? else {
            return Err(EntityOperationError::Busy);
        };
        match read_json_if_exists::<EntityDeletionRecord>(&record_path)? {
            Some(record) if record.phase == DeletionPhase::Deleted => {
                // 已删稳定拒绝：不得重复清理/复活。
                Err(EntityOperationError::Deleted)
            }
            _ => Ok(EntityDeletionLease {
                _lock: lock,
                scope: scope.clone(),
            }),
        }
    }

    /// 低层迟到写防御（plan §0.2）：**try-shared，锁争用立即返回
    /// `Conflict{kind:"entity_deleting"}`，禁止阻塞等待 scope**；取得后锁内
    /// 复核墓碑（purging/deleted 均稳定拒绝）。
    pub fn with_live_write<T>(
        &self,
        scope: &EntityScope,
        operation: impl FnOnce() -> Result<T, ProductStoreError>,
    ) -> Result<T, ProductStoreError> {
        let record_path = entity_deletion_record_path(&self.paths, scope)?;
        let lock_path = scope_lock_path(&self.paths, scope)?;
        let Some(_lock) = StableFileLock::try_acquire_shared(&lock_path)? else {
            return Err(ProductStoreError::Conflict {
                kind: "entity_deleting",
                id: scope.entity_id.clone(),
            });
        };
        if read_json_if_exists::<EntityDeletionRecord>(&record_path)?.is_some() {
            return Err(ProductStoreError::Conflict {
                kind: "entity_deleting",
                id: scope.entity_id.clone(),
            });
        }
        operation()
    }

    /// issue 关系短锁（删除协调层消费；run 不持关系锁）。固定锁序：scope
    /// 集合（外层排序）→ repo resource 锁 → **本关系短锁** → session/object；
    /// 调用方必须已按序持有上游锁，禁止持 session/object 锁后首次取
    /// scope/resource 再进入。短临界区阻塞独占，不可重入。
    pub fn with_issue_relations<T>(
        &self,
        project_id: &str,
        issue_id: &str,
        operation: impl FnOnce() -> Result<T, ProductStoreError>,
    ) -> Result<T, ProductStoreError> {
        let lock_path = issue_relations_lock_path(&self.paths, project_id, issue_id)?;
        let _guard = StableFileLock::acquire_exclusive(&lock_path)?;
        operation()
    }

    /// repo resource 写准入：非阻塞独占短 lease；争用 Busy；锁 inode 在 purge 外。
    pub fn try_acquire_shared_resource_write(
        &self,
        key: &SharedResourceKey,
    ) -> Result<SharedResourceWriteLease, EntityOperationError> {
        Ok(SharedResourceWriteLease {
            _lock: self.try_acquire_resource_lock(key)?,
            key: key.clone(),
        })
    }

    /// repo resource 清理准入：非阻塞独占短 lease，与写者互斥；争用 Busy。
    pub fn try_acquire_shared_resource_cleanup(
        &self,
        key: &SharedResourceKey,
    ) -> Result<SharedResourceCleanupLease, EntityOperationError> {
        Ok(SharedResourceCleanupLease {
            _lock: self.try_acquire_resource_lock(key)?,
            key: key.clone(),
        })
    }

    /// 身份预留（D1.3a 消费）：同存活 scope 幂等；删除中→entity_deleting；
    /// 墓碑（purging/deleted）拒复用。
    pub fn reserve_entity_identity(&self, scope: &EntityScope) -> Result<(), ProductStoreError> {
        let record_path = entity_deletion_record_path(&self.paths, scope)?;
        let lock_path = scope_lock_path(&self.paths, scope)?;
        let reservation_path = entity_identity_reservation_path(&self.paths, scope)?;
        let Some(_lock) = StableFileLock::try_acquire_shared(&lock_path)? else {
            return Err(ProductStoreError::Conflict {
                kind: "entity_deleting",
                id: scope.entity_id.clone(),
            });
        };
        if read_json_if_exists::<EntityDeletionRecord>(&record_path)?.is_some() {
            return Err(ProductStoreError::Conflict {
                kind: "entity_deleted",
                id: scope.entity_id.clone(),
            });
        }
        match read_json_if_exists::<EntityIdentityReservation>(&reservation_path)? {
            None => write_json(
                &reservation_path,
                &EntityIdentityReservation {
                    scope: scope.clone(),
                },
            ),
            Some(existing) if &existing.scope == scope => Ok(()),
            Some(_) => Err(ProductStoreError::Conflict {
                kind: "entity_identity_reserved",
                id: scope.entity_id.clone(),
            }),
        }
    }

    /// 全项目 workspace session ID 分配：现全项目 session 最大序号与耐久高水位
    /// 取 max 再 +1；分配即持久化高水位，已分配（含已删实体）ID 不复用。
    pub fn allocate_workspace_session_id(&self) -> Result<String, ProductStoreError> {
        let marker_path = self.paths.state_root().join("entity-session-high-water.json");
        let lock_path = self.paths.state_root().join("entity-session-high-water.lock");
        with_exact_exclusive_lock(&lock_path, || {
            let scan_max = scan_workspace_session_sequence(&self.paths.projects_root())?;
            let marker_high = read_json_if_exists::<WorkspaceSessionHighWater>(&marker_path)?
                .map(|marker| marker.high_water)
                .unwrap_or(0);
            let next = scan_max.max(marker_high) + 1;
            write_json(
                &marker_path,
                &WorkspaceSessionHighWater { high_water: next },
            )?;
            Ok(next_sequential_id("workspace_session", next - 1))
        })
    }

    /// 会话身份绑定（D1.3a 消费）：只 ID/scope；同身份幂等，异 scope 冲突。
    pub fn bind_workspace_session_identity(
        &self,
        session_id: &str,
        scope: &EntityScope,
    ) -> Result<(), ProductStoreError> {
        let binding_path = session_identity_binding_path(&self.paths, session_id)?;
        scope.validate()?;
        let bind_lock = binding_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(|parent| parent.join(format!(".{session_id}.bind.lock")))
            .unwrap_or_else(|| PathBuf::from(format!(".{session_id}.bind.lock")));
        with_exact_exclusive_lock(&bind_lock, || {
            match read_json_if_exists::<WorkspaceSessionIdentityBinding>(&binding_path)? {
                None => write_json(
                    &binding_path,
                    &WorkspaceSessionIdentityBinding {
                        scope: scope.clone(),
                    },
                ),
                Some(existing) if &existing.scope == scope => Ok(()),
                Some(_) => Err(ProductStoreError::Conflict {
                    kind: "workspace_session_binding",
                    id: session_id.to_string(),
                }),
            }
        })
    }

    /// 反查会话归属：missing/歧义/墓碑 fail-closed（不做 live 授权）。
    pub fn scope_for_workspace_session(&self, session_id: &str) -> Result<EntityScope, ProductStoreError> {
        let binding_path = session_identity_binding_path(&self.paths, session_id)?;
        let binding =
            read_json_if_exists::<WorkspaceSessionIdentityBinding>(&binding_path)?.ok_or_else(
                || ProductStoreError::NotFound {
                    kind: "workspace_session_binding",
                    id: session_id.to_string(),
                },
            )?;
        let record_path = entity_deletion_record_path(&self.paths, &binding.scope)?;
        if read_json_if_exists::<EntityDeletionRecord>(&record_path)?.is_some() {
            return Err(ProductStoreError::Conflict {
                kind: "entity_deleted",
                id: session_id.to_string(),
            });
        }
        Ok(binding.scope)
    }

    /// list-by-owner（§0.5）：纯 ID/scope 身份索引（bind 落盘）∪ owned-index
    /// session_ids；允许 purging/deleted scope 查询（不能误当 live 授权）；
    /// entity root 缺失仍可反查。
    pub fn list_workspace_session_ids_by_owner(
        &self,
        scope: &EntityScope,
    ) -> Result<Vec<String>, ProductStoreError> {
        scope.validate()?;
        let mut ids = BTreeSet::new();
        let bindings_root = self.paths.state_root().join("entity-identity").join("sessions");
        for entry in child_directory_files(&bindings_root)? {
            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else {
                continue;
            };
            let Some(session_id) = name.strip_suffix(".json") else {
                continue;
            };
            match read_json_if_exists::<WorkspaceSessionIdentityBinding>(&entry.path())? {
                Some(binding) if &binding.scope == scope => {
                    ids.insert(session_id.to_string());
                }
                _ => {}
            }
        }
        let owned = crate::product::entity_deletion_store::EntityDeletionStore::new(
            self.paths.clone(),
        )
        .list_owned_artifact_ids(scope)?;
        ids.extend(owned.session_ids);
        Ok(ids.into_iter().collect())
    }

    /// list-by-owner（§0.5）：纯 ID/scope 占用索引
    /// （`EntityDeletionStore::index_owned_artifact_ids` 落盘）的 work_item_ids；
    /// root/pending 均缺后仍可反查。
    pub fn list_child_entity_ids_by_owner(&self, scope: &EntityScope) -> Result<Vec<String>, ProductStoreError> {
        scope.validate()?;
        Ok(
            crate::product::entity_deletion_store::EntityDeletionStore::new(
                self.paths.clone(),
            )
            .list_owned_artifact_ids(scope)?
            .work_item_ids,
        )
    }

    /// scope 锁争用分类：无墓碑→Busy（entity_busy fail-fast）；purging→
    /// Deleting；deleted→Deleted；读取错误→Store。
    fn scope_contention_error(&self, record_path: &Path) -> EntityOperationError {
        match read_json_if_exists::<EntityDeletionRecord>(record_path) {
            Ok(None) => EntityOperationError::Busy,
            Ok(Some(record)) => record.phase.into(),
            Err(error) => EntityOperationError::Store(error),
        }
    }

    /// repo resource 非阻塞独占短锁（写者/清理互斥，锁 inode 在 purge 外）。
    fn try_acquire_resource_lock(
        &self,
        key: &SharedResourceKey,
    ) -> Result<StableFileLock, EntityOperationError> {
        let lock_path = shared_resource_lock_path(&self.paths, key)?;
        match StableFileLock::try_acquire_exclusive(&lock_path)? {
            Some(lock) => Ok(lock),
            None => Err(EntityOperationError::Busy),
        }
    }
}

impl From<DeletionPhase> for EntityOperationError {
    fn from(phase: DeletionPhase) -> Self {
        match phase {
            DeletionPhase::Purging => Self::Deleting,
            DeletionPhase::Deleted => Self::Deleted,
        }
    }
}

/// 全项目 workspace session 最大序号扫描（文件名 `workspace_session_{n}.json`）。
fn scan_workspace_session_sequence(projects_root: &Path) -> Result<usize, ProductStoreError> {
    let mut max_sequence = 0usize;
    for project_path in child_directories(projects_root)? {
        let issues_root = project_path.join("issues");
        for issue_path in child_directories(&issues_root)? {
            let sessions_root = issue_path.join("workspace-sessions");
            for entry in child_directory_files(&sessions_root)? {
                let Some(sequence) = entry
                    .file_name()
                    .to_str()
                    .and_then(|name| name.strip_suffix(".json"))
                    .and_then(|stem| stem.strip_prefix("workspace_session_"))
                    .and_then(|suffix| suffix.parse::<usize>().ok())
                else {
                    continue;
                };
                max_sequence = max_sequence.max(sequence);
            }
        }
    }
    Ok(max_sequence)
}

/// 子目录枚举：NotFound=空；其余 IO 错误必报。
fn child_directories(root: &Path) -> Result<Vec<PathBuf>, ProductStoreError> {
    Ok(child_entries(root)?
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .map(|entry| entry.path())
        .collect())
}

/// 目录内文件枚举：NotFound=空；其余 IO 错误必报。
fn child_directory_files(root: &Path) -> Result<Vec<std::fs::DirEntry>, ProductStoreError> {
    Ok(child_entries(root)?
        .filter(|entry| entry.file_type().map(|kind| kind.is_file()).unwrap_or(false))
        .collect())
}

fn child_entries(root: &Path) -> Result<impl Iterator<Item = std::fs::DirEntry>, ProductStoreError> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(Vec::new().into_iter());
        }
        Err(error) => {
            return Err(ProductStoreError::Io(format!(
                "read {}: {error}",
                root.display()
            )))
        }
    };
    Ok(entries
        .map(|entry| {
            entry.map_err(|error| {
                ProductStoreError::Io(format!("read entry in {}: {error}", root.display()))
            })
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter())
}

fn scope_lock_path(paths: &ProductAppPaths, scope: &EntityScope) -> Result<PathBuf, ProductStoreError> {
    Ok(issue_entity_operations_root(paths, scope)?
        .join("scope-locks")
        .join(scope.kind.path_segment())
        .join(format!("{}.lock", scope.entity_id)))
}

fn issue_relations_lock_path(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
) -> Result<PathBuf, ProductStoreError> {
    crate::product::json_store::validate_relative_id(project_id)?;
    crate::product::json_store::validate_relative_id(issue_id)?;
    Ok(paths
        .issue_lifecycle_root(project_id, issue_id)
        .join("entity-operations")
        .join("relations.lock"))
}

fn shared_resource_lock_path(
    paths: &ProductAppPaths,
    key: &SharedResourceKey,
) -> Result<PathBuf, ProductStoreError> {
    crate::product::json_store::validate_relative_id(&key.project_id)?;
    crate::product::json_store::validate_relative_id(&key.issue_id)?;
    let segment = key
        .repository_id
        .map(|repository_id| repository_id.0.to_string())
        .unwrap_or_else(|| "issue-shared".to_string());
    Ok(paths
        .issue_lifecycle_root(&key.project_id, &key.issue_id)
        .join("entity-operations")
        .join("resource-locks")
        .join(format!("{segment}.lock")))
}

fn entity_identity_reservation_path(
    paths: &ProductAppPaths,
    scope: &EntityScope,
) -> Result<PathBuf, ProductStoreError> {
    Ok(issue_entity_operations_root(paths, scope)?
        .join("identity")
        .join("entities")
        .join(scope.kind.path_segment())
        .join(format!("{}.json", scope.entity_id)))
}

fn session_identity_binding_path(paths: &ProductAppPaths, session_id: &str) -> Result<PathBuf, ProductStoreError> {
    crate::product::json_store::validate_relative_id(session_id)?;
    Ok(paths
        .state_root()
        .join("entity-identity")
        .join("sessions")
        .join(format!("{session_id}.json")))
}

#[derive(Debug, Serialize, Deserialize)]
struct EntityIdentityReservation {
    #[serde(flatten)]
    scope: EntityScope,
}

#[derive(Debug, Serialize, Deserialize)]
struct WorkspaceSessionIdentityBinding {
    #[serde(flatten)]
    scope: EntityScope,
}

#[derive(Debug, Serialize, Deserialize)]
struct WorkspaceSessionHighWater {
    high_water: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    use crate::product::coding_attempt_store::locking::{
        ExclusiveFileLock, register_exact_lock_attempt_hook,
    };
    use crate::product::entity_deletion_store::{
        DeletionEntityKind, EntityDeletionStore, EntityDeletionManifest,
    };

    fn test_paths() -> (tempfile::TempDir, ProductAppPaths) {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = ProductAppPaths::new(temp.path().join("aria"));
        (temp, paths)
    }

    fn plan_scope(entity_id: &str) -> EntityScope {
        EntityScope::new("project_0001", "issue_0001", DeletionEntityKind::Plan, entity_id)
    }

    #[test]
    fn cascade_d1_0_delete_exclusive_blocks_live_write_nonblocking() {
        let (_temp, paths) = test_paths();
        let operations = EntityOperationStore::new(paths.clone());
        let scope = plan_scope("plan_0001");

        let lease = operations.try_acquire_delete(&scope).expect("delete lease");

        // 第二个删除竞争者：try-exclusive 争用 → Busy，不等待。
        assert!(matches!(
            operations.try_acquire_delete(&scope),
            Err(EntityOperationError::Busy)
        ));

        // 删除独占持有中：with_live_write 立即 Conflict(entity_deleting)，超时断言非阻塞。
        let content_dir = paths
            .issue_root("project_0001", "issue_0001")
            .join("workspace-sessions");
        let started = Instant::now();
        let error = operations
            .with_live_write::<()>(&scope, || {
                panic!("operation must not run while deletion holds the scope")
            })
            .expect_err("live write must be rejected");
        assert!(started.elapsed() < std::time::Duration::from_millis(500));
        assert!(matches!(
            error,
            ProductStoreError::Conflict {
                kind: "entity_deleting",
                ..
            }
        ));
        // 零目录创建：被拒迟到写不得创建任何实体内容目录。
        assert!(!content_dir.exists());

        // 墓碑未写时运行启动 → Busy（独占争用、无墓碑）。
        assert!(matches!(
            operations.acquire_run(&scope),
            Err(EntityOperationError::Busy)
        ));

        // 墓碑写为 purging 后运行启动 → Deleting。
        let deletions = EntityDeletionStore::new(paths.clone());
        deletions.begin(&lease).expect("begin purging");
        assert!(matches!(
            operations.acquire_run(&scope),
            Err(EntityOperationError::Deleting)
        ));
    }

    #[test]
    fn cascade_d1_0_scope_tombstone_recheck_after_complete() {
        let (_temp, paths) = test_paths();
        let operations = EntityOperationStore::new(paths.clone());
        let deletions = EntityDeletionStore::new(paths.clone());
        let scope = plan_scope("plan_0002");

        let lease = operations.try_acquire_delete(&scope).expect("delete lease");
        deletions.begin(&lease).expect("begin");
        let completed = deletions.complete(&lease).expect("complete");
        assert_eq!(completed.phase, DeletionPhase::Deleted);
        drop(lease);

        // 锁已释放，但墓碑复核仍稳定拒绝迟到写与运行/删除准入。
        assert!(matches!(
            operations.with_live_write::<()>(&scope, || Ok(())),
            Err(ProductStoreError::Conflict {
                kind: "entity_deleting",
                ..
            })
        ));
        assert!(matches!(
            operations.acquire_run(&scope),
            Err(EntityOperationError::Deleted)
        ));
        assert!(matches!(
            operations.try_acquire_delete(&scope),
            Err(EntityOperationError::Deleted)
        ));
    }

    #[test]
    fn cascade_d1_0_run_lease_and_admitted_write_under_object_lock() {
        let (_temp, paths) = test_paths();
        let operations = EntityOperationStore::new(paths.clone());
        let scope = plan_scope("plan_0003");

        let run = operations.acquire_run(&scope).expect("first run lease");
        let second_run = operations.acquire_run(&scope).expect("shared run lease coexists");

        // run 持共享时删除 try-exclusive → Busy（entity_busy fail-fast）。
        assert!(matches!(
            operations.try_acquire_delete(&scope),
            Err(EntityOperationError::Busy)
        ));
        drop(second_run);

        // 既有 session/object 锁内：消费已持 run lease 的已准入 helper 提交，
        // 不重取 scope（hook 断言零次 scope 锁尝试）。
        let session_record = paths
            .issue_root("project_0001", "issue_0001")
            .join("workspace-sessions")
            .join("workspace_session_0001.json");
        let object_lock = ExclusiveFileLock::acquire(&session_record).expect("object lock");
        let scope_lock = scope_lock_path(&paths, &scope).expect("scope lock path");
        let (_hook, attempts) = register_exact_lock_attempt_hook(&scope_lock);
        let artifact = paths.root().join("admitted-artifact.json");
        run.with_admitted_write(|| {
            write_json(&artifact, &serde_json::json!({"committed": true}))
        })
        .expect("admitted write commits under object lock");
        let committed: serde_json::Value =
            crate::product::json_store::read_json(&artifact).expect("read admitted artifact");
        assert_eq!(committed["committed"], serde_json::json!(true));
        assert!(
            attempts.try_recv().is_err(),
            "admitted write must not re-acquire the scope lock"
        );
        drop(object_lock);
    }

    #[test]
    fn cascade_d1_0_shared_resource_writer_cleanup_exclusion_and_stable_inode() {
        let (_temp, paths) = test_paths();
        let operations = EntityOperationStore::new(paths.clone());
        let repository_id = LogicalRepositoryId(
            uuid::Uuid::parse_str("11111111-1111-1111-1111-111111111111").expect("uuid"),
        );
        let repo_key = SharedResourceKey {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: Some(repository_id),
        };
        let legacy_key = SharedResourceKey {
            repository_id: None,
            ..repo_key.clone()
        };

        let writer = operations
            .try_acquire_shared_resource_write(&repo_key)
            .expect("resource writer lease");
        // writer/cleanup 互斥；writer/writer 亦互斥（均非阻塞独占短 lease）。
        assert!(matches!(
            operations.try_acquire_shared_resource_cleanup(&repo_key),
            Err(EntityOperationError::Busy)
        ));
        assert!(matches!(
            operations.try_acquire_shared_resource_write(&repo_key),
            Err(EntityOperationError::Busy)
        ));
        // 异键（Legacy issue shared 路径）不互斥。
        let legacy_writer = operations
            .try_acquire_shared_resource_write(&legacy_key)
            .expect("legacy resource writer lease");

        let lock_path = shared_resource_lock_path(&paths, &repo_key).expect("resource lock path");
        let inode_before = lock_file_inode(&lock_path);
        drop(writer);

        let cleanup = operations
            .try_acquire_shared_resource_cleanup(&repo_key)
            .expect("cleanup lease after writer release");
        let inode_after = lock_file_inode(&lock_path);
        assert_eq!(
            inode_before, inode_after,
            "resource lock inode must be stable across acquisitions (never unlinked by purge)"
        );
        drop(cleanup);
        drop(legacy_writer);
    }

    #[test]
    fn cascade_d1_0_owner_index_lists_after_root_missing() {
        let (_temp, paths) = test_paths();
        let operations = EntityOperationStore::new(paths.clone());
        let deletions = EntityDeletionStore::new(paths.clone());
        let scope_a = EntityScope::new(
            "project_0001",
            "issue_0001",
            DeletionEntityKind::Story,
            "story_0001",
        );
        let scope_b = EntityScope::new(
            "project_0001",
            "issue_0001",
            DeletionEntityKind::Story,
            "story_0002",
        );

        // 身份绑定：同身份幂等，异 scope 冲突。
        operations
            .bind_workspace_session_identity("workspace_session_0001", &scope_a)
            .expect("bind session 1");
        operations
            .bind_workspace_session_identity("workspace_session_0001", &scope_a)
            .expect("bind session 1 idempotent");
        operations
            .bind_workspace_session_identity("workspace_session_0002", &scope_a)
            .expect("bind session 2");
        operations
            .bind_workspace_session_identity("workspace_session_0003", &scope_b)
            .expect("bind session 3 to scope b");
        assert!(matches!(
            operations.bind_workspace_session_identity("workspace_session_0001", &scope_b),
            Err(ProductStoreError::Conflict { .. })
        ));

        // 纯 ID 占用索引（删除重入/归属反查的固定来源）。
        let lease = operations.try_acquire_delete(&scope_a).expect("delete lease");
        let manifest = EntityDeletionManifest {
            session_ids: vec![
                "workspace_session_0001".to_string(),
                "workspace_session_0002".to_string(),
            ],
            work_item_ids: vec!["work_item_a1".to_string(), "work_item_a2".to_string()],
            ..EntityDeletionManifest::empty(scope_a.clone())
        };
        deletions
            .index_owned_artifact_ids(&lease, &manifest)
            .expect("index owned artifacts");

        // entity root 记录从未存在（root 缺失）：list-by-owner 仍可反查且不跨 scope。
        assert_eq!(
            operations
                .list_workspace_session_ids_by_owner(&scope_a)
                .expect("list sessions for scope a"),
            vec![
                "workspace_session_0001".to_string(),
                "workspace_session_0002".to_string(),
            ]
        );
        assert_eq!(
            operations
                .list_workspace_session_ids_by_owner(&scope_b)
                .expect("list sessions for scope b"),
            vec!["workspace_session_0003".to_string()]
        );
        assert_eq!(
            operations
                .list_child_entity_ids_by_owner(&scope_a)
                .expect("list children for scope a"),
            vec!["work_item_a1".to_string(), "work_item_a2".to_string()]
        );
        assert!(operations
            .list_child_entity_ids_by_owner(&scope_b)
            .expect("list children for scope b")
            .is_empty());

        // 反查：missing fail-closed。
        assert_eq!(
            operations
                .scope_for_workspace_session("workspace_session_0001")
                .expect("scope for session 1"),
            scope_a
        );
        assert!(matches!(
            operations.scope_for_workspace_session("workspace_session_9999"),
            Err(ProductStoreError::NotFound { .. })
        ));

        // 墓碑完成后：scope_for fail-closed；list-by-owner 仍可查（准入事实，非 live 授权）。
        deletions.begin(&lease).expect("begin");
        deletions.complete(&lease).expect("complete");
        assert!(matches!(
            operations.scope_for_workspace_session("workspace_session_0001"),
            Err(ProductStoreError::Conflict { .. })
        ));
        assert_eq!(
            operations
                .list_workspace_session_ids_by_owner(&scope_a)
                .expect("list sessions for deleted scope a"),
            vec![
                "workspace_session_0001".to_string(),
                "workspace_session_0002".to_string(),
            ]
        );
    }

    #[test]
    fn cascade_d1_0_identity_high_water_and_no_reuse() {
        let (_temp, paths) = test_paths();
        let operations = EntityOperationStore::new(paths.clone());
        let scope = plan_scope("plan_0004");

        // 空库：从 1 开始。
        assert_eq!(
            operations
                .allocate_workspace_session_id()
                .expect("first allocation"),
            "workspace_session_0001"
        );
        assert_eq!(
            operations
                .allocate_workspace_session_id()
                .expect("second allocation"),
            "workspace_session_0002"
        );

        // 现存 session 最大序号（全项目扫描）高于高水位 → 扫描胜出。
        let sessions_root = paths
            .issue_root("project_0002", "issue_0007")
            .join("workspace-sessions");
        std::fs::create_dir_all(&sessions_root).expect("create sessions root");
        std::fs::write(
            sessions_root.join("workspace_session_0007.json"),
            "{}",
        )
        .expect("seed session file");
        assert_eq!(
            operations
                .allocate_workspace_session_id()
                .expect("allocation above scan max"),
            "workspace_session_0008"
        );

        // 扫描来源被清（模拟已删 session）：耐久高水位防止 ID 复用。
        std::fs::remove_file(sessions_root.join("workspace_session_0007.json"))
            .expect("remove seeded session");
        assert_eq!(
            operations
                .allocate_workspace_session_id()
                .expect("allocation above durable high water"),
            "workspace_session_0009"
        );
        // 新 store 实例从耐久高水位继续。
        assert_eq!(
            EntityOperationStore::new(paths.clone())
                .allocate_workspace_session_id()
                .expect("fresh store continues high water"),
            "workspace_session_0010"
        );

        // 实体身份预留：同存活 scope 幂等；墓碑拒复用。
        operations
            .reserve_entity_identity(&scope)
            .expect("reserve identity");
        operations
            .reserve_entity_identity(&scope)
            .expect("reserve identity idempotent");
        let deletions = EntityDeletionStore::new(paths.clone());
        let lease = operations.try_acquire_delete(&scope).expect("delete lease");
        deletions.begin(&lease).expect("begin");
        deletions.complete(&lease).expect("complete");
        drop(lease);
        assert!(matches!(
            operations.reserve_entity_identity(&scope),
            Err(ProductStoreError::Conflict { .. })
        ));
    }

    #[test]
    fn cascade_d1_0_issue_relations_lock_passthrough_and_validation() {
        let (_temp, paths) = test_paths();
        let operations = EntityOperationStore::new(paths.clone());

        let value = operations
            .with_issue_relations("project_0001", "issue_0001", || Ok(42))
            .expect("relations operation");
        assert_eq!(value, 42);
        assert!(matches!(
            operations.with_issue_relations("../evil", "issue_0001", || Ok(1)),
            Err(ProductStoreError::PathEscape(_))
        ));
    }

    #[cfg(unix)]
    fn lock_file_inode(path: &Path) -> (u64, u64) {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::metadata(path).expect("lock file metadata");
        (metadata.dev(), metadata.ino())
    }

    #[cfg(not(unix))]
    fn lock_file_inode(_path: &Path) -> (u64, u64) {
        (0, 0)
    }
}
