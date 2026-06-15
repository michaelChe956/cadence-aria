use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Serialize, de::DeserializeOwned};

#[derive(Debug, thiserror::Error)]
pub enum ProductStoreError {
    #[error("product_store_io: {0}")]
    Io(String),
    #[error("product_store_json: {0}")]
    Json(String),
    #[error("product_store_not_found: {kind} {id}")]
    NotFound { kind: &'static str, id: String },
    #[error("product_store_path_escape: {0}")]
    PathEscape(String),
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, ProductStoreError> {
    let file = std::fs::File::open(path)
        .map_err(|error| ProductStoreError::Io(format!("open {}: {error}", path.display())))?;
    serde_json::from_reader(file).map_err(|error| ProductStoreError::Json(error.to_string()))
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), ProductStoreError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            ProductStoreError::Io(format!("create {}: {error}", parent.display()))
        })?;
    }

    let temp_path = temp_path_for(path);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .map_err(|error| {
            ProductStoreError::Io(format!("create {}: {error}", temp_path.display()))
        })?;

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
