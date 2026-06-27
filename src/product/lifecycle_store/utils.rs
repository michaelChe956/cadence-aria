use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id};
use crate::product::models::{SpecVersionRecord, WorkspaceSessionRecord};

pub(crate) fn list_json_records<T: DeserializeOwned>(
    path: &Path,
) -> Result<Vec<T>, ProductStoreError> {
    let entries = json_file_paths(path)?;

    let mut records = Vec::with_capacity(entries.len());
    for entry in entries {
        records.push(read_json(&entry)?);
    }
    Ok(records)
}

pub(crate) fn count_json_files(path: &Path) -> Result<usize, ProductStoreError> {
    Ok(json_file_paths(path)?.len())
}

pub(crate) fn json_file_paths(path: &Path) -> Result<Vec<PathBuf>, ProductStoreError> {
    if !path_exists(path)? {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    for entry in fs::read_dir(path)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))?
    {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", path.display()))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            ProductStoreError::Io(format!(
                "read {} entry type: {error}",
                entry.path().display()
            ))
        })?;
        let entry_path = entry.path();
        if file_type.is_file()
            && entry_path.extension().and_then(|value| value.to_str()) == Some("json")
        {
            entries.push(entry_path);
        }
    }
    entries.sort();
    Ok(entries)
}

pub(crate) fn list_workspace_session_records(
    path: &Path,
) -> Result<Vec<WorkspaceSessionRecord>, ProductStoreError> {
    let entries = workspace_session_file_paths(path)?;

    let mut records = Vec::with_capacity(entries.len());
    for entry in entries {
        if let Some(record) = read_workspace_session_record(&entry)? {
            records.push(record);
        }
    }
    Ok(records)
}

pub(crate) fn workspace_session_file_paths(path: &Path) -> Result<Vec<PathBuf>, ProductStoreError> {
    Ok(json_file_paths(path)?
        .into_iter()
        .filter(|path| workspace_session_file_stem(path).is_some())
        .collect())
}

pub(crate) fn read_workspace_session_record(
    path: &Path,
) -> Result<Option<WorkspaceSessionRecord>, ProductStoreError> {
    let Some(file_id) = workspace_session_file_stem(path) else {
        return Ok(None);
    };
    let session: WorkspaceSessionRecord = read_json(path)?;
    if session.id == file_id {
        Ok(Some(session))
    } else {
        Ok(None)
    }
}

pub(crate) fn workspace_session_file_stem(path: &Path) -> Option<&str> {
    let stem = path.file_stem()?.to_str()?;
    let suffix = stem.strip_prefix("workspace_session_")?;
    if suffix.is_empty() {
        return None;
    }
    Some(stem)
}

pub(crate) fn child_directories(path: &Path) -> Result<Vec<PathBuf>, ProductStoreError> {
    if !path_exists(path)? {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    for entry in fs::read_dir(path)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))?
    {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", path.display()))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            ProductStoreError::Io(format!(
                "read {} entry type: {error}",
                entry.path().display()
            ))
        })?;
        if file_type.is_dir() {
            entries.push(entry.path());
        }
    }
    entries.sort();
    Ok(entries)
}

pub(crate) fn next_version_number(records: &[SpecVersionRecord]) -> Result<u32, ProductStoreError> {
    records
        .iter()
        .map(|record| record.version)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| ProductStoreError::Io("version sequence overflow".to_string()))
}

pub(crate) fn ensure_target_absent(path: &Path) -> Result<(), ProductStoreError> {
    if path_exists(path)? {
        return Err(ProductStoreError::Io(format!(
            "refuse to overwrite {}",
            path.display()
        )));
    }
    Ok(())
}

pub(crate) fn delete_required_file(
    path: &Path,
    kind: &'static str,
    id: &str,
) -> Result<(), ProductStoreError> {
    if !path_is_regular_file(path)? {
        return Err(ProductStoreError::NotFound {
            kind,
            id: id.to_string(),
        });
    }
    remove_file_if_exists(path)
}

pub(crate) fn remove_file_if_exists(path: &Path) -> Result<(), ProductStoreError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ProductStoreError::Io(format!(
            "remove {}: {error}",
            path.display()
        ))),
    }
}

pub(crate) fn remove_dir_all_if_exists(path: &Path) -> Result<(), ProductStoreError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ProductStoreError::Io(format!(
            "remove {}: {error}",
            path.display()
        ))),
    }
}

pub(crate) fn path_is_regular_file(path: &Path) -> Result<bool, ProductStoreError> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(metadata.is_file()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(ProductStoreError::Io(format!(
            "metadata {}: {error}",
            path.display()
        ))),
    }
}

pub(crate) fn validate_relative_ids(values: &[String]) -> Result<(), ProductStoreError> {
    for value in values {
        validate_relative_id(value)?;
    }
    Ok(())
}

pub(crate) fn path_exists(path: &Path) -> Result<bool, ProductStoreError> {
    path.try_exists()
        .map_err(|error| ProductStoreError::Io(format!("try_exists {}: {error}", path.display())))
}
