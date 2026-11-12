use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Serialize, de::DeserializeOwned};

use crate::product::audit_snapshot_budget::AuditSnapshotBudgetFailure;

#[derive(Debug, thiserror::Error)]
pub enum ProductStoreError {
    #[error("product_store_io: {0}")]
    Io(String),
    #[error("product_store_json: {0}")]
    Json(String),
    #[error("product_store_not_found: {kind} {id}")]
    NotFound { kind: &'static str, id: String },
    #[error("product_store_ambiguous: {kind} {id}")]
    Ambiguous { kind: &'static str, id: String },
    #[error("product_store_conflict: {kind} {id}")]
    Conflict { kind: &'static str, id: String },
    #[error("product_store_invalid_record: {kind} {reason}")]
    InvalidRecord { kind: &'static str, reason: String },
    #[error("product_store_identity_mismatch: {kind} {id}")]
    IdentityMismatch { kind: &'static str, id: String },
    #[error("product_store_path_escape: {0}")]
    PathEscape(String),
    /// The audit snapshot exceeded its byte / entry budget. The payload keeps
    /// the structured breakdown (limits, charged amounts, top groups, per
    /// member contribution) so the failure copy can render an actionable
    /// message; the English line stays machine-readable for existing
    /// consumers.
    #[error("product_store_audit_snapshot_budget_exceeded: {0}")]
    AuditSnapshotBudgetExceeded(AuditSnapshotBudgetFailure),
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, ProductStoreError> {
    let file = std::fs::File::open(path)
        .map_err(|error| ProductStoreError::Io(format!("open {}: {error}", path.display())))?;
    serde_json::from_reader(file).map_err(|error| ProductStoreError::Json(error.to_string()))
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), ProductStoreError> {
    write_json_impl(path, value, false)
}

/// 与 [`write_json`] 相同的原子写流程，但 unix 下临时文件在**创建时**即 0600：
/// 明文密钥等敏感内容从写入到 rename 全程属主可读写，不存在 0644 暴露窗口
/// （即使进程在 rename 之后、任何后续 chmod 之前崩溃，落盘文件也已是 0600）。
/// Windows 无 POSIX 权限位，`mode` 不生效，行为与 [`write_json`] 一致。
pub fn write_json_owner_only<T: Serialize>(
    path: &Path,
    value: &T,
) -> Result<(), ProductStoreError> {
    write_json_impl(path, value, true)
}

fn write_json_impl<T: Serialize>(
    path: &Path,
    value: &T,
    owner_only: bool,
) -> Result<(), ProductStoreError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            ProductStoreError::Io(format!("create {}: {error}", parent.display()))
        })?;
    }

    let temp_path = temp_path_for(path);
    let mut file = open_temp_file(&temp_path, owner_only)?;

    serde_json::to_writer_pretty(&mut file, value).map_err(|error| {
        ProductStoreError::Json(format!(
            "write {} via {}: {error}",
            path.display(),
            temp_path.display()
        ))
    })?;
    file.flush().map_err(|error| {
        ProductStoreError::Io(format!("flush {}: {error}", temp_path.display()))
    })?;
    file.sync_all()
        .map_err(|error| ProductStoreError::Io(format!("sync {}: {error}", temp_path.display())))?;
    drop(file);

    rename_temp_file(&temp_path, path)
}

fn open_temp_file(temp_path: &Path, owner_only: bool) -> Result<std::fs::File, ProductStoreError> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        if owner_only {
            use std::os::unix::fs::OpenOptionsExt;
            // create_new + mode(0600)：权限位随 open(2) 创建一次生效，内容写入前即收紧。
            options.mode(0o600);
        }
    }
    // Windows 等平台无 POSIX 权限位，owner_only 不适用（由调用方按需另行处理）。
    #[cfg(not(unix))]
    {
        let _ = owner_only;
    }
    options
        .open(temp_path)
        .map_err(|error| ProductStoreError::Io(format!("create {}: {error}", temp_path.display())))
}

fn temp_path_for(path: &Path) -> PathBuf {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .map(|value| value.to_string_lossy())
        .unwrap_or_else(|| "product-store.json".into());
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);

    parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        timestamp
    ))
}

fn rename_temp_file(temp_path: &Path, target_path: &Path) -> Result<(), ProductStoreError> {
    match std::fs::rename(temp_path, target_path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let cleanup_result = std::fs::remove_file(temp_path);
            let cleanup_context = cleanup_result
                .err()
                .map(|cleanup_error| {
                    format!("; cleanup {} failed: {cleanup_error}", temp_path.display())
                })
                .unwrap_or_default();
            Err(ProductStoreError::Io(format!(
                "rename {} to {}: {error}{cleanup_context}",
                temp_path.display(),
                target_path.display()
            )))
        }
    }
}

pub fn validate_relative_id(value: &str) -> Result<(), ProductStoreError> {
    let path = Path::new(value);
    if value.is_empty()
        || value.contains('/')
        || value.contains('\\')
        || path.components().any(|component| {
            matches!(
                component,
                Component::Prefix(_) | Component::RootDir | Component::ParentDir
            )
        })
    {
        return Err(ProductStoreError::PathEscape(value.to_string()));
    }

    Ok(())
}

pub fn validate_relative_artifact_ref(value: &str) -> Result<(), ProductStoreError> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::Prefix(_) | Component::RootDir | Component::ParentDir
            )
        })
    {
        return Err(ProductStoreError::PathEscape(value.to_string()));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// 恒定序列化失败的探针值：让 serde_json 在临时文件创建之后、写入中途报错。
    /// （不能依赖 f64::NAN：当前 serde_json 版本会把它序列化为 null 并成功返回。）
    struct FailSerialize;

    impl serde::Serialize for FailSerialize {
        fn serialize<S: serde::Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("simulated mid-write failure"))
        }
    }

    #[test]
    fn owner_only_temp_file_is_created_with_mode_0600() {
        // 观测手段：序列化在临时文件创建之后失败并提前返回（此路径不清理临时文件）
        // ⇒ 可直接读取留在磁盘上的临时文件的权限位，断言「创建即 0600」：
        // 明文 key 在写入期间不存在 0644 暴露窗口。
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("secret.json");

        let result = write_json_owner_only(&target, &FailSerialize);

        assert!(result.is_err(), "序列化必须失败以留下临时文件供观测");
        let temp_path = std::fs::read_dir(dir.path())
            .expect("read dir")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.extension().is_some_and(|ext| ext == "tmp"))
            .expect("失败的 owner-only 写入应把临时文件留在目标目录");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&temp_path)
                .expect("temp metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "owner-only 临时文件必须在创建时即 0600");
        }
    }

    #[test]
    fn owner_only_write_persists_target_with_mode_0600() {
        // 端到端：owner-only 写成功后目标文件为 0600（rename 保留临时文件权限位，
        // 目标从落盘那一刻起就不存在 0644 窗口）。
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("secret.json");

        write_json_owner_only(&target, &serde_json::json!({"api_key": "sk-plain"}))
            .expect("owner-only write");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&target)
                .expect("target metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "owner-only 写出的目标文件必须为 0600");
        }
    }
}
