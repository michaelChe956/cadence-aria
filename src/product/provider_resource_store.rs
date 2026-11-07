//! 批2 Task 5:provider 运行资源登记的 durable store(C1.3 / D S6 / B4 /
//! REQ-ENV-HOST-01 / R11)。
//!
//! 持久化范式复用 [`crate::product::json_store`](tmp+flush+sync+rename)与
//! 现有 exclusive lock 范式(`coding_attempt_store::locking`),不新建第二套
//! IO 语义。目录布局(每 scope 一个锁域):
//!
//! ```text
//! <root>/scopes/<scope_id>/<run_id>/<boot_identity>/
//!   resource-mutations          # 锁目标文件(flock 派生 .resource-mutations.lock)
//!   resources/<resource_id>.json
//! ```
//!
//! 合同(与 `cross_cutting::provider_resources` 共同冻结):
//! - `reserve` 先于任何启动副作用落盘(create-new 语义:同 key 同内容幂等,
//!   同 key 异内容 conflict fail-closed);
//! - `confirm_started` 只把真实身份固化进已存在的 Reserved 记录(身份核对
//!   预留事实,不凭空登记);cleanup cursor 单调前进、永不回退;
//! - `deregister` 幂等(重复清理/补偿重放安全);
//! - 主键 `(scope_id, run_id, boot_identity)` 即路径身份:跨 boot 的登记
//!   由恢复消费方(T14/T17/T21)经 [`ProviderResourceStore::scope_keys`]
//!   枚举接管,不在此处隐式合并。
// 生产消费链:经 `cross_cutting::provider_resources` 由 T6/T7/T14/T17/T21
// 接线;接线完成前仅合同测试消费(前瞻载体范式)。
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::cross_cutting::provider_resources::{
    CleanupStep, ProviderResourceRecord, ResourceScopeKey, StartedIdentity,
};
use crate::product::coding_attempt_store::locking::with_exclusive_lock;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

/// 同 key 重复 reserve 且内容不同时的冲突判别码前缀。
pub(crate) const PROVIDER_RESOURCE_RESERVE_CONFLICT: &str = "provider_resource_reserve_conflict";

/// 确认/游标推进时 durable 记录与调用方预留事实不符的判别码前缀。
pub(crate) const PROVIDER_RESOURCE_IDENTITY_MISMATCH: &str = "provider_resource_identity_mismatch";

/// 游标回退(或非法跳步)的判别码前缀。
pub(crate) const PROVIDER_RESOURCE_CURSOR_REGRESSION: &str = "provider_resource_cursor_regression";

/// 确认写入的测试 failpoint(T5 红绿证据:「启动后确认写失败会停子进程」;
/// 与 `coding_attempt_store` 的 delivery-mark failpoint 同款范式,但为
/// store 实例级——并行测试互不干扰)。
pub(crate) struct ProviderResourceStore {
    root: PathBuf,
    /// 实例级 confirm 写 failpoint(仅测试装配)。
    #[cfg(test)]
    confirm_failpoint: std::sync::atomic::AtomicBool,
}

impl ProviderResourceStore {
    pub(crate) fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            #[cfg(test)]
            confirm_failpoint: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// 仅测试:注入 confirm 持久写失败(红绿证据用)。
    #[cfg(test)]
    pub(crate) fn set_confirm_write_failpoint(&self, enabled: bool) {
        self.confirm_failpoint
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
    }

    /// durable 预留登记:先于启动副作用落盘。同 key 同记录幂等;同 key
    /// 异内容 conflict。整个读-判-写在 scope 锁域内完成。
    pub(crate) fn reserve(&self, record: &ProviderResourceRecord) -> Result<(), ProductStoreError> {
        let path = self.resource_path(&record.key, &record.resource_id)?;
        with_exclusive_lock(&self.lock_target(&record.key)?, || {
            if path.is_file() {
                let existing: ProviderResourceRecord = read_json(&path)?;
                if existing.matches_registration(record) {
                    return Ok(());
                }
                return Err(ProductStoreError::Conflict {
                    kind: "provider_resource_record",
                    id: record.resource_id.clone(),
                }
                .map_details(|details| {
                    format!("{PROVIDER_RESOURCE_RESERVE_CONFLICT}: {details}")
                }));
            }
            write_json(&path, record)
        })
    }

    /// 固化真实身份:记录必须已 Reserved 存在,且 kind/launch_intent/labels/
    /// 私有路径与预留事实一致;游标 Reserved→Confirmed(同身份重放幂等)。
    pub(crate) fn confirm_started(
        &self,
        key: &ResourceScopeKey,
        resource_id: &str,
        identity: &StartedIdentity,
        expected_launch_intent: &str,
        expected_labels: &BTreeMap<String, String>,
        expected_private_path: Option<&str>,
    ) -> Result<ProviderResourceRecord, ProductStoreError> {
        #[cfg(test)]
        if self
            .confirm_failpoint
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            // 200ms 宽限:让已 spawn 的 fixture 先落 pid/标记文件,测试才能
            // 在确认失败补偿后以 /proc 观察真实静止(仅测试装配)。
            std::thread::sleep(std::time::Duration::from_millis(200));
            return Err(ProductStoreError::Io(
                "confirm_write_failpoint: injected durable-write failure".to_string(),
            ));
        }
        let path = self.resource_path(key, resource_id)?;
        with_exclusive_lock(&self.lock_target(key)?, || {
            let mut record: ProviderResourceRecord = read_json(&path)?;
            if !record.matches_registration(&ProviderResourceRecord {
                key: key.clone(),
                resource_id: resource_id.to_string(),
                kind: record.kind,
                launch_intent: expected_launch_intent.to_string(),
                reserved_labels: expected_labels.clone(),
                started_identity: record.started_identity.clone(),
                canonical_private_path: expected_private_path.map(str::to_string),
                cleanup_cursor: record.cleanup_cursor,
                reserved_at: record.reserved_at.clone(),
                updated_at: record.updated_at.clone(),
            }) {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "provider_resource_record",
                    id: resource_id.to_string(),
                }
                .map_details(|details| {
                    format!("{PROVIDER_RESOURCE_IDENTITY_MISMATCH}: {details}")
                }));
            }
            match record.started_identity.as_ref() {
                Some(existing) if existing != identity => {
                    return Err(ProductStoreError::Conflict {
                        kind: "provider_resource_record",
                        id: resource_id.to_string(),
                    });
                }
                _ => {}
            }
            record.started_identity = Some(identity.clone());
            // F3(审查修复轮):重放不得回退游标——仅 Reserved(< Confirmed)
            // 才写 Confirmed,Quiesced/Removed 前进态原样保留。
            if record.cleanup_cursor < CleanupStep::Confirmed {
                record.cleanup_cursor = CleanupStep::Confirmed;
            }
            record.updated_at = Utc::now().to_rfc3339();
            write_json(&path, &record)?;
            Ok(record)
        })
    }

    /// cleanup cursor 单调推进(Quiesced/Removed);拒绝回退,同值幂等。
    pub(crate) fn advance_cleanup_cursor(
        &self,
        key: &ResourceScopeKey,
        resource_id: &str,
        step: CleanupStep,
    ) -> Result<ProviderResourceRecord, ProductStoreError> {
        let path = self.resource_path(key, resource_id)?;
        with_exclusive_lock(&self.lock_target(key)?, || {
            let mut record: ProviderResourceRecord = read_json(&path)?;
            if step < record.cleanup_cursor {
                return Err(ProductStoreError::Conflict {
                    kind: "provider_resource_record",
                    id: resource_id.to_string(),
                }
                .map_details(|details| {
                    format!("{PROVIDER_RESOURCE_CURSOR_REGRESSION}: {details}")
                }));
            }
            if step == record.cleanup_cursor {
                return Ok(record);
            }
            record.cleanup_cursor = step;
            record.updated_at = Utc::now().to_rfc3339();
            write_json(&path, &record)?;
            Ok(record)
        })
    }

    /// 解除登记(清理/补偿完成):幂等——记录已不存在视为成功。
    pub(crate) fn deregister(
        &self,
        key: &ResourceScopeKey,
        resource_id: &str,
    ) -> Result<(), ProductStoreError> {
        let path = self.resource_path(key, resource_id)?;
        with_exclusive_lock(&self.lock_target(key)?, || {
            match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(ProductStoreError::Io(format!(
                    "remove {}: {error}",
                    path.display()
                ))),
            }
        })
    }

    pub(crate) fn load_record(
        &self,
        key: &ResourceScopeKey,
        resource_id: &str,
    ) -> Result<Option<ProviderResourceRecord>, ProductStoreError> {
        let path = self.resource_path(key, resource_id)?;
        if !path.is_file() {
            return Ok(None);
        }
        with_exclusive_lock(&self.lock_target(key)?, || Ok(Some(read_json(&path)?)))
    }

    /// scope 全部登记项(=T14/T17 交接的完整资源 manifest 来源)。
    pub(crate) fn list_scope(
        &self,
        key: &ResourceScopeKey,
    ) -> Result<Vec<ProviderResourceRecord>, ProductStoreError> {
        let resources_dir = self.resources_dir(key)?;
        with_exclusive_lock(&self.lock_target(key)?, || {
            let mut records: Vec<ProviderResourceRecord> = Vec::new();
            let entries = match std::fs::read_dir(&resources_dir) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(records),
                Err(error) => {
                    return Err(ProductStoreError::Io(format!(
                        "read {}: {error}",
                        resources_dir.display()
                    )));
                }
            };
            for entry in entries {
                let entry = entry.map_err(|error| {
                    ProductStoreError::Io(format!("read {}: {error}", resources_dir.display()))
                })?;
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    continue;
                };
                if !name.ends_with(".json") {
                    continue;
                }
                records.push(read_json(&entry.path())?);
            }
            records.sort_by(|a, b| a.resource_id.cmp(&b.resource_id));
            Ok(records)
        })
    }

    /// 枚举全部登记 scope(恢复消费方接管 stale boot 的入口)。只返回仍有
    /// 登记文件的 scope key。
    pub(crate) fn scope_keys(&self) -> Result<Vec<ResourceScopeKey>, ProductStoreError> {
        let scopes_root = self.root.join("scopes");
        let mut keys = Vec::new();
        let Some(scope_entries) = read_dir_opt(&scopes_root)? else {
            return Ok(keys);
        };
        for scope_entry in scope_entries {
            let scope_entry = scope_entry.map_err(|error| {
                ProductStoreError::Io(format!("read {}: {error}", scopes_root.display()))
            })?;
            if !scope_entry
                .file_type()
                .map(|kind| kind.is_dir())
                .unwrap_or(false)
            {
                continue;
            }
            let Some(scope_id) = scope_entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let run_root = scope_entry.path();
            let Some(run_entries) = read_dir_opt(&run_root)? else {
                continue;
            };
            for run_entry in run_entries {
                let run_entry = run_entry.map_err(|error| {
                    ProductStoreError::Io(format!("read {}: {error}", run_root.display()))
                })?;
                if !run_entry
                    .file_type()
                    .map(|kind| kind.is_dir())
                    .unwrap_or(false)
                {
                    continue;
                }
                let Some(run_id) = run_entry.file_name().to_str().map(str::to_string) else {
                    continue;
                };
                let boot_root = run_entry.path();
                let Some(boot_entries) = read_dir_opt(&boot_root)? else {
                    continue;
                };
                for boot_entry in boot_entries {
                    let boot_entry = boot_entry.map_err(|error| {
                        ProductStoreError::Io(format!("read {}: {error}", boot_root.display()))
                    })?;
                    if !boot_entry
                        .file_type()
                        .map(|kind| kind.is_dir())
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    let Some(boot_identity) = boot_entry.file_name().to_str().map(str::to_string)
                    else {
                        continue;
                    };
                    let resources_dir = boot_entry.path().join("resources");
                    if !dir_has_json(&resources_dir)? {
                        continue;
                    }
                    keys.push(ResourceScopeKey {
                        scope_id: scope_id.clone(),
                        run_id: run_id.clone(),
                        boot_identity,
                    });
                }
            }
        }
        keys.sort();
        Ok(keys)
    }

    fn scope_dir(&self, key: &ResourceScopeKey) -> Result<PathBuf, ProductStoreError> {
        validate_key_component(&key.scope_id)?;
        validate_key_component(&key.run_id)?;
        validate_key_component(&key.boot_identity)?;
        Ok(self
            .root
            .join("scopes")
            .join(&key.scope_id)
            .join(&key.run_id)
            .join(&key.boot_identity))
    }

    fn resources_dir(&self, key: &ResourceScopeKey) -> Result<PathBuf, ProductStoreError> {
        Ok(self.scope_dir(key)?.join("resources"))
    }

    fn resource_path(
        &self,
        key: &ResourceScopeKey,
        resource_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(resource_id)?;
        Ok(self.resources_dir(key)?.join(format!("{resource_id}.json")))
    }

    fn lock_target(&self, key: &ResourceScopeKey) -> Result<PathBuf, ProductStoreError> {
        Ok(self.scope_dir(key)?.join("resource-mutations"))
    }
}

fn validate_key_component(value: &str) -> Result<(), ProductStoreError> {
    validate_relative_id(value)
}

fn read_dir_opt(directory: &Path) -> Result<Option<std::fs::ReadDir>, ProductStoreError> {
    match std::fs::read_dir(directory) {
        Ok(entries) => Ok(Some(entries)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ProductStoreError::Io(format!(
            "read {}: {error}",
            directory.display()
        ))),
    }
}

fn dir_has_json(directory: &Path) -> Result<bool, ProductStoreError> {
    Ok(read_dir_opt(directory)?
        .map(|entries| {
            entries.filter_map(Result::ok).any(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.ends_with(".json"))
            })
        })
        .unwrap_or(false))
}

impl ProductStoreError {
    /// 保持枚举判别码不变、追加稳定前缀与上下文到 details。
    fn map_details(self, render: impl FnOnce(String) -> String) -> Self {
        match self {
            Self::Io(details) => Self::Io(render(details)),
            Self::Json(details) => Self::Json(render(details)),
            Self::NotFound { kind, id } => Self::NotFound {
                kind,
                id: render(id),
            },
            Self::Ambiguous { kind, id } => Self::Ambiguous {
                kind,
                id: render(id),
            },
            Self::Conflict { kind, id } => Self::Conflict {
                kind,
                id: render(id),
            },
            Self::InvalidRecord { kind, reason } => Self::InvalidRecord {
                kind,
                reason: render(reason),
            },
            Self::IdentityMismatch { kind, id } => Self::IdentityMismatch {
                kind,
                id: render(id),
            },
            Self::PathEscape(value) => Self::PathEscape(render(value)),
            // 审计预算失败是结构化载荷：它自带稳定码与 `Display`，不做字符串
            // 前缀渲染，直接透传。
            Self::AuditSnapshotBudgetExceeded(failure) => {
                Self::AuditSnapshotBudgetExceeded(failure)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cross_cutting::provider_resources::OwnedResourceKind;

    fn t5_dir() -> tempfile::TempDir {
        let root = Path::new("/tmp/aria-bug45/t5/store");
        std::fs::create_dir_all(root).expect("create /tmp/aria-bug45/t5/store");
        tempfile::tempdir_in(root).expect("store fixture dir")
    }

    fn key(boot: &str) -> ResourceScopeKey {
        ResourceScopeKey {
            scope_id: "scope_run_0001".to_string(),
            run_id: "run_0001".to_string(),
            boot_identity: boot.to_string(),
        }
    }

    fn record(key: &ResourceScopeKey, resource_id: &str) -> ProviderResourceRecord {
        ProviderResourceRecord {
            key: key.clone(),
            resource_id: resource_id.to_string(),
            kind: OwnedResourceKind::Directory,
            launch_intent: "private scratch dir".to_string(),
            reserved_labels: BTreeMap::from([
                ("aria.scope_id".to_string(), key.scope_id.clone()),
                ("aria.run_id".to_string(), key.run_id.clone()),
            ]),
            started_identity: None,
            canonical_private_path: Some("/tmp/aria-bug45/t5/fixture".to_string()),
            cleanup_cursor: CleanupStep::Reserved,
            reserved_at: "2026-10-17T00:00:00Z".to_string(),
            updated_at: "2026-10-17T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn reserve_persists_and_roundtrips() {
        let dir = t5_dir();
        let store = ProviderResourceStore::new(dir.path());
        let key = key("boot_a");
        store
            .reserve(&record(&key, "res_0001"))
            .expect("reserve must persist");
        let loaded = store
            .load_record(&key, "res_0001")
            .expect("load reserved")
            .expect("record present");
        assert_eq!(loaded.resource_id, "res_0001");
        assert_eq!(loaded.cleanup_cursor, CleanupStep::Reserved);
        assert_eq!(store.list_scope(&key).unwrap().len(), 1);
    }

    #[test]
    fn reserve_is_idempotent_for_identical_record_and_conflicts_on_drift() {
        let dir = t5_dir();
        let store = ProviderResourceStore::new(dir.path());
        let key = key("boot_a");
        store.reserve(&record(&key, "res_0001")).unwrap();
        store
            .reserve(&record(&key, "res_0001"))
            .expect("idempotent replay");

        let drifted = ProviderResourceRecord {
            launch_intent: "different intent".to_string(),
            ..record(&key, "res_0001")
        };
        let error = store
            .reserve(&drifted)
            .expect_err("conflicting re-reserve must fail closed");
        assert!(
            error
                .to_string()
                .contains(PROVIDER_RESOURCE_RESERVE_CONFLICT),
            "conflict must carry stable reason code: {error}"
        );
    }

    #[test]
    fn confirm_started_freezes_identity_and_requires_matching_reservation() {
        let dir = t5_dir();
        let store = ProviderResourceStore::new(dir.path());
        let key = key("boot_a");
        let reserved = record(&key, "res_0001");
        store.reserve(&reserved).unwrap();

        let confirmed = store
            .confirm_started(
                &key,
                "res_0001",
                &StartedIdentity::Directory {
                    canonical_path: "/tmp/aria-bug45/t5/fixture".to_string(),
                },
                &reserved.launch_intent,
                &reserved.reserved_labels,
                reserved.canonical_private_path.as_deref(),
            )
            .expect("confirm freezes real identity");
        assert_eq!(confirmed.cleanup_cursor, CleanupStep::Confirmed);

        // 未预留的资源不能凭空确认。
        let error = store
            .confirm_started(
                &key,
                "res_unknown",
                &StartedIdentity::Directory {
                    canonical_path: "/x".to_string(),
                },
                "intent",
                &BTreeMap::new(),
                None,
            )
            .expect_err("confirm without reservation must fail");
        assert!(matches!(error, ProductStoreError::Io(_)));
    }

    #[test]
    fn cleanup_cursor_only_moves_forward() {
        let dir = t5_dir();
        let store = ProviderResourceStore::new(dir.path());
        let key = key("boot_a");
        let reserved = record(&key, "res_0001");
        store.reserve(&reserved).unwrap();
        store
            .confirm_started(
                &key,
                "res_0001",
                &StartedIdentity::Directory {
                    canonical_path: "/tmp/aria-bug45/t5/fixture".to_string(),
                },
                &reserved.launch_intent,
                &reserved.reserved_labels,
                reserved.canonical_private_path.as_deref(),
            )
            .unwrap();

        store
            .advance_cleanup_cursor(&key, "res_0001", CleanupStep::Quiesced)
            .expect("forward advance");
        store
            .advance_cleanup_cursor(&key, "res_0001", CleanupStep::Quiesced)
            .expect("same-step replay is idempotent");
        let error = store
            .advance_cleanup_cursor(&key, "res_0001", CleanupStep::Confirmed)
            .expect_err("cursor regression must fail closed");
        assert!(
            error
                .to_string()
                .contains(PROVIDER_RESOURCE_CURSOR_REGRESSION),
            "regression must carry stable reason code: {error}"
        );
    }

    #[test]
    fn deregister_is_idempotent() {
        let dir = t5_dir();
        let store = ProviderResourceStore::new(dir.path());
        let key = key("boot_a");
        store.reserve(&record(&key, "res_0001")).unwrap();
        store.deregister(&key, "res_0001").unwrap();
        store
            .deregister(&key, "res_0001")
            .expect("repeated deregister is a no-op");
        assert!(store.load_record(&key, "res_0001").unwrap().is_none());
    }

    #[test]
    fn scope_keys_enumerates_only_scopes_with_records() {
        let dir = t5_dir();
        let store = ProviderResourceStore::new(dir.path());
        assert!(store.scope_keys().unwrap().is_empty());
        store.reserve(&record(&key("boot_a"), "res_0001")).unwrap();
        store.reserve(&record(&key("boot_b"), "res_0002")).unwrap();
        let keys = store.scope_keys().unwrap();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&key("boot_a")));
        assert!(keys.contains(&key("boot_b")));
    }

    #[test]
    fn path_escape_in_key_or_resource_id_is_rejected() {
        let dir = t5_dir();
        let store = ProviderResourceStore::new(dir.path());
        let mut bad = record(&key("boot_a"), "res_0001");
        bad.key.scope_id = "../escape".to_string();
        let error = store.reserve(&bad).expect_err("path escape must fail");
        assert!(matches!(error, ProductStoreError::PathEscape(_)));
    }
}
