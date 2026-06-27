use super::PlanningUnitError;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub(crate) struct DirectorySnapshot {
    existed: bool,
    files: BTreeMap<PathBuf, Vec<u8>>,
}

pub(crate) fn snapshot_directory(path: &Path) -> Result<DirectorySnapshot, PlanningUnitError> {
    let mut files = BTreeMap::new();
    if !path.exists() {
        return Ok(DirectorySnapshot {
            existed: false,
            files,
        });
    }
    collect_snapshot_files(path, path, &mut files)?;
    Ok(DirectorySnapshot {
        existed: true,
        files,
    })
}

fn collect_snapshot_files(
    root: &Path,
    path: &Path,
    files: &mut BTreeMap<PathBuf, Vec<u8>>,
) -> Result<(), PlanningUnitError> {
    for entry in std::fs::read_dir(path)
        .map_err(|error| PlanningUnitError::Io(format!("read {}: {error}", path.display())))?
    {
        let entry =
            entry.map_err(|error| PlanningUnitError::Io(format!("read dir entry: {error}")))?;
        let path = entry.path();
        if path.is_dir() {
            collect_snapshot_files(root, &path, files)?;
        } else if path.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|error| PlanningUnitError::Io(error.to_string()))?
                .to_path_buf();
            let content = std::fs::read(&path).map_err(|error| {
                PlanningUnitError::Io(format!("read {}: {error}", path.display()))
            })?;
            files.insert(relative, content);
        }
    }
    Ok(())
}

pub(crate) fn restore_directory_snapshot(
    path: &Path,
    snapshot: &DirectorySnapshot,
) -> Result<(), PlanningUnitError> {
    if path.exists() {
        std::fs::remove_dir_all(path).map_err(|error| {
            PlanningUnitError::Io(format!("remove {}: {error}", path.display()))
        })?;
    }
    if !snapshot.existed {
        return Ok(());
    }
    std::fs::create_dir_all(path)
        .map_err(|error| PlanningUnitError::Io(format!("create {}: {error}", path.display())))?;
    for (relative, content) in &snapshot.files {
        let target = path.join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                PlanningUnitError::Io(format!("create {}: {error}", parent.display()))
            })?;
        }
        std::fs::write(&target, content).map_err(|error| {
            PlanningUnitError::Io(format!("write {}: {error}", target.display()))
        })?;
    }
    Ok(())
}
