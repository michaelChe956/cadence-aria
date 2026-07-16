use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::WorkItemPlanLineage;

mod dependency;
mod handoff;
mod paths;
mod plan;
mod presentation;
mod projection;
mod repair;
mod work_item;

#[derive(Debug, Clone)]
pub struct WorkItemRevisionStore {
    paths: ProductAppPaths,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PlanScope {
    plan_id: String,
    project_id: String,
    issue_id: String,
}

impl WorkItemRevisionStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    fn ensure_plan_scope(
        &self,
        lineage: &WorkItemPlanLineage,
    ) -> Result<WorkItemPlanLineage, ProductStoreError> {
        validate_relative_id(&lineage.project_id)?;
        validate_relative_id(&lineage.issue_id)?;
        validate_relative_id(&lineage.id)?;
        let stored = self.get_plan_lineage(&lineage.project_id, &lineage.issue_id, &lineage.id)?;
        if stored.project_id != lineage.project_id
            || stored.issue_id != lineage.issue_id
            || stored.id != lineage.id
        {
            return Err(identity_mismatch("work_item_plan_lineage", &lineage.id));
        }
        Ok(stored)
    }

    fn scope_for_plan_id(&self, plan_id: &str) -> Result<PlanScope, ProductStoreError> {
        validate_relative_id(plan_id)?;
        let mut scopes = Vec::new();
        for project_path in child_directories(&self.plan_scopes_root(plan_id))? {
            let Some(path_project_id) = project_path.file_name().and_then(|value| value.to_str())
            else {
                continue;
            };
            for scope_path in json_file_paths(&project_path)? {
                let Some(path_issue_id) = scope_path.file_stem().and_then(|value| value.to_str())
                else {
                    continue;
                };
                let scope: PlanScope = read_json(&scope_path)?;
                if scope.plan_id != plan_id
                    || scope.project_id != path_project_id
                    || scope.issue_id != path_issue_id
                {
                    return Err(identity_mismatch("work_item_plan_scope", plan_id));
                }
                validate_relative_id(&scope.project_id)?;
                validate_relative_id(&scope.issue_id)?;
                scopes.push(scope);
            }
        }
        match scopes.as_slice() {
            [scope] => Ok(scope.clone()),
            [] => Err(ProductStoreError::NotFound {
                kind: "work_item_plan_scope",
                id: plan_id.to_string(),
            }),
            _ => Err(ProductStoreError::Ambiguous {
                kind: "work_item_plan_scope",
                id: plan_id.to_string(),
            }),
        }
    }
}

fn identity_mismatch(kind: &'static str, id: &str) -> ProductStoreError {
    ProductStoreError::IdentityMismatch {
        kind,
        id: id.to_string(),
    }
}

fn read_required_json<T: DeserializeOwned>(
    path: &Path,
    kind: &'static str,
    id: &str,
) -> Result<T, ProductStoreError> {
    if !path_exists(path)? {
        return Err(ProductStoreError::NotFound {
            kind,
            id: id.to_string(),
        });
    }
    read_json(path)
}

fn write_immutable<T>(
    path: &Path,
    kind: &'static str,
    id: &str,
    value: &T,
) -> Result<(), ProductStoreError>
where
    T: Serialize + DeserializeOwned + PartialEq,
{
    if path_exists(path)? {
        let existing: T = read_json(path)?;
        if existing == *value {
            return Ok(());
        }
        return Err(identity_mismatch(kind, id));
    }
    write_json(path, value)
}

fn path_exists(path: &Path) -> Result<bool, ProductStoreError> {
    match fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(ProductStoreError::Io(format!(
            "metadata {}: {error}",
            path.display()
        ))),
    }
}

fn json_file_paths(path: &Path) -> Result<Vec<PathBuf>, ProductStoreError> {
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
        let entry_path = entry.path();
        if entry
            .file_type()
            .map_err(|error| {
                ProductStoreError::Io(format!("read {} entry type: {error}", entry_path.display()))
            })?
            .is_file()
            && entry_path.extension().and_then(|value| value.to_str()) == Some("json")
        {
            entries.push(entry_path);
        }
    }
    entries.sort();
    Ok(entries)
}

fn child_directories(path: &Path) -> Result<Vec<PathBuf>, ProductStoreError> {
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
        let entry_path = entry.path();
        if entry
            .file_type()
            .map_err(|error| {
                ProductStoreError::Io(format!("read {} entry type: {error}", entry_path.display()))
            })?
            .is_dir()
        {
            entries.push(entry_path);
        }
    }
    entries.sort();
    Ok(entries)
}

#[cfg(test)]
mod tests;
