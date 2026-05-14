use std::path::{Component, Path};

use serde::{Serialize, de::DeserializeOwned};

#[derive(Debug, thiserror::Error)]
pub enum ProductStoreError {
    #[error("product_store_io: {0}")]
    Io(String),
    #[error("product_store_json: {0}")]
    Json(String),
    #[error("product_store_path_escape: {0}")]
    PathEscape(String),
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, ProductStoreError> {
    let file = std::fs::File::open(path)
        .map_err(|error| ProductStoreError::Io(format!("open {}: {error}", path.display())))?;
    serde_json::from_reader(file).map_err(|error| ProductStoreError::Json(error.to_string()))
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), ProductStoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            ProductStoreError::Io(format!("create {}: {error}", parent.display()))
        })?;
    }
    let file = std::fs::File::create(path)
        .map_err(|error| ProductStoreError::Io(format!("create {}: {error}", path.display())))?;
    serde_json::to_writer_pretty(file, value)
        .map_err(|error| ProductStoreError::Json(error.to_string()))
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
