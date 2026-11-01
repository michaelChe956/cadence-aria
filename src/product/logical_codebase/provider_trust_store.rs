//! REQ-REG-14 durable facts：Codex/Kimi trust ownership、前后摘要、
//! 等待/审计事实。
//!
//! 记录落在 per-LC scope（`logical-codebases/{lc_id}/provider-trust/`，
//! legacy 别名 codebase 保持 `logical-codebase/provider-trust/`），全部为
//! byte-stable JSON；变更在 scope 级 `.provider-trust.lock` 上串行化。
//! 审计事实只含 digest/path/key 等非敏感内容，绝不记录凭据。

use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::locking::with_exact_exclusive_lock;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

/// 归属证明：本 LC 是否拥有（并因此可撤销）某条用户级 trust。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderTrustOwnershipRecord {
    pub lc_id: String,
    pub provider: String,
    pub canonical_root: PathBuf,
    pub trust_key: String,
    /// `false` 表示该条目是用户自建、被本 LC 原样复用，不可被本 LC 撤销。
    pub owned: bool,
    pub before_digest: Option<String>,
    pub after_digest: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// fail-closed 可重试等待事实（REQ-REG-12/REQ-REG-14）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderTrustWaitingRecord {
    pub lc_id: String,
    pub provider: String,
    pub canonical_root: PathBuf,
    pub trust_key: String,
    pub operation_id: Option<String>,
    pub reason_code: String,
    pub message: String,
    pub retry_action: String,
    pub recorded_at: String,
}

/// 跨工作区副作用审计事实：root/provider/key/动作/前后摘要/时间戳。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderTrustAuditRecord {
    pub audit_id: String,
    pub lc_id: String,
    pub operation_id: Option<String>,
    pub provider: String,
    pub canonical_root: PathBuf,
    pub trust_key: String,
    /// ensure | verify | revoke | gate_waiting
    pub action: String,
    /// registered | replay | reuse_user_entry | conflict | trusted |
    /// untrusted | missing | removed | not_owned | entry_missing |
    /// digest_mismatch | <reason_code>
    pub outcome: String,
    pub before_digest: Option<String>,
    pub after_digest: Option<String>,
    pub recorded_at: String,
}

/// Per-LC scoped trust durable-fact store.
#[derive(Debug, Clone)]
pub struct ProviderTrustStore {
    paths: ProductAppPaths,
    lc_id: Option<String>,
}

impl ProviderTrustStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths, lc_id: None }
    }

    /// Scopes every trust fact to one logical codebase subtree（与
    /// `AggregateInitializationOperationStore::for_lc` 同一布局规则）。
    pub fn for_lc(paths: ProductAppPaths, lc_id: impl Into<String>) -> Self {
        Self {
            paths,
            lc_id: Some(lc_id.into()),
        }
    }

    fn scope_root(&self, project_id: &str) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        crate::product::logical_codebase::lc_scope_root(&self.paths, project_id, &self.lc_id)
    }

    fn trust_root(&self, project_id: &str) -> Result<PathBuf, ProductStoreError> {
        Ok(self.scope_root(project_id)?.join("provider-trust"))
    }

    fn lock_path(&self, project_id: &str) -> Result<PathBuf, ProductStoreError> {
        Ok(self.scope_root(project_id)?.join(".provider-trust.lock"))
    }

    fn ownership_path(
        &self,
        project_id: &str,
        provider: &str,
        trust_key: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(provider)?;
        Ok(self
            .trust_root(project_id)?
            .join("registrations")
            .join(provider)
            .join(format!("{}.json", key_hash(trust_key))))
    }

    fn waiting_path(
        &self,
        project_id: &str,
        provider: &str,
        trust_key: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(provider)?;
        Ok(self
            .trust_root(project_id)?
            .join("waiting")
            .join(provider)
            .join(format!("{}.json", key_hash(trust_key))))
    }

    fn audit_root(&self, project_id: &str) -> Result<PathBuf, ProductStoreError> {
        Ok(self.trust_root(project_id)?.join("audit"))
    }

    /// Upsert one ownership record. Identical replays write nothing.
    pub fn record_ownership(
        &self,
        project_id: &str,
        record: ProviderTrustOwnershipRecord,
    ) -> Result<(), ProductStoreError> {
        with_exact_exclusive_lock(&self.lock_path(project_id)?, || {
            let path = self.ownership_path(project_id, &record.provider, &record.trust_key)?;
            if path.exists() {
                let existing: ProviderTrustOwnershipRecord = read_json(&path)?;
                if existing == record {
                    return Ok(());
                }
            }
            write_json(&path, &record)
        })
    }

    pub fn load_ownership(
        &self,
        project_id: &str,
        provider: &str,
        trust_key: &str,
    ) -> Result<Option<ProviderTrustOwnershipRecord>, ProductStoreError> {
        let path = self.ownership_path(project_id, provider, trust_key)?;
        if !path.exists() {
            return Ok(None);
        }
        let record: ProviderTrustOwnershipRecord = read_json(&path)?;
        if record.provider != provider || record.trust_key != trust_key {
            return Err(ProductStoreError::InvalidRecord {
                kind: "provider_trust_ownership",
                reason: format!(
                    "record at {} does not match requested provider/key",
                    path.display()
                ),
            });
        }
        Ok(Some(record))
    }

    pub fn delete_ownership(
        &self,
        project_id: &str,
        provider: &str,
        trust_key: &str,
    ) -> Result<(), ProductStoreError> {
        with_exact_exclusive_lock(&self.lock_path(project_id)?, || {
            let path = self.ownership_path(project_id, provider, trust_key)?;
            if path.exists() {
                std::fs::remove_file(&path).map_err(|error| {
                    ProductStoreError::Io(format!("remove {}: {error}", path.display()))
                })?;
            }
            Ok(())
        })
    }

    /// Persist（覆盖）one waiting fact; deterministic path makes retries
    /// idempotent.
    pub fn record_waiting(
        &self,
        project_id: &str,
        record: ProviderTrustWaitingRecord,
    ) -> Result<(), ProductStoreError> {
        with_exact_exclusive_lock(&self.lock_path(project_id)?, || {
            let path = self.waiting_path(project_id, &record.provider, &record.trust_key)?;
            write_json(&path, &record)
        })
    }

    /// Remove the waiting fact once the provider is trusted again.
    pub fn clear_waiting(
        &self,
        project_id: &str,
        provider: &str,
        trust_key: &str,
    ) -> Result<(), ProductStoreError> {
        with_exact_exclusive_lock(&self.lock_path(project_id)?, || {
            let path = self.waiting_path(project_id, provider, trust_key)?;
            if path.exists() {
                std::fs::remove_file(&path).map_err(|error| {
                    ProductStoreError::Io(format!("remove {}: {error}", path.display()))
                })?;
            }
            Ok(())
        })
    }

    /// Lists the scope's waiting facts, oldest first.
    pub fn list_waiting(
        &self,
        project_id: &str,
    ) -> Result<Vec<ProviderTrustWaitingRecord>, ProductStoreError> {
        let root = self.trust_root(project_id)?.join("waiting");
        let mut records = Vec::new();
        for provider_root in read_child_dirs(&root)? {
            for path in read_json_dir(&provider_root)? {
                let record: ProviderTrustWaitingRecord = read_json(&path)?;
                records.push(record);
            }
        }
        records.sort_by(|left, right| left.recorded_at.cmp(&right.recorded_at));
        Ok(records)
    }

    /// Appends one immutable audit fact (filename carries a monotonic
    /// timestamp prefix plus a uuid).
    pub fn append_audit(
        &self,
        project_id: &str,
        record: ProviderTrustAuditRecord,
    ) -> Result<(), ProductStoreError> {
        with_exact_exclusive_lock(&self.lock_path(project_id)?, || {
            let path = self.audit_root(project_id)?.join(format!(
                "{}-{}.json",
                sort_key_nanos(),
                record.audit_id
            ));
            write_json(&path, &record)
        })
    }

    /// Lists the scope's audit facts, oldest first.
    pub fn list_audit(
        &self,
        project_id: &str,
    ) -> Result<Vec<ProviderTrustAuditRecord>, ProductStoreError> {
        let mut records = Vec::new();
        for path in read_json_dir(&self.audit_root(project_id)?)? {
            let record: ProviderTrustAuditRecord = read_json(&path)?;
            records.push(record);
        }
        // read_json_dir 已按文件名（19 位纳秒前缀）升序，无需再排序。
        Ok(records)
    }
}

fn key_hash(trust_key: &str) -> String {
    let digest = format!("{:x}", Sha256::digest(trust_key.as_bytes()));
    digest[..16].to_string()
}

fn sort_key_nanos() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{nanos:019}")
}

fn read_child_dirs(root: &std::path::Path) -> Result<Vec<PathBuf>, ProductStoreError> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut children = Vec::new();
    for entry in std::fs::read_dir(root)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", root.display())))?
    {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", root.display()))
        })?;
        let path = entry.path();
        if path.is_dir() {
            children.push(path);
        }
    }
    children.sort();
    Ok(children)
}

fn read_json_dir(root: &std::path::Path) -> Result<Vec<PathBuf>, ProductStoreError> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(root)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", root.display())))?
    {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", root.display()))
        })?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) == Some("json") {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

/// Convenience constructor for audit ids.
pub fn new_audit_id() -> String {
    format!("provider_trust_audit_{}", Uuid::new_v4().simple())
}

/// Shared timestamp helper (rfc3339, matching sibling stores).
pub(crate) fn trust_now() -> String {
    Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    // store 读写面由 provider_trust.rs 的 gate 级测试覆盖（ownership/waiting/
    // audit 全部经 HomeBackedProviderTrustRegistry 往返断言）。
}
