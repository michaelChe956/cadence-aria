use std::fs;
use std::path::Path;

use chrono::Utc;

use crate::product::app_paths::ProductAppPaths;
use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::logical_codebase::LogicalCodebaseStore;
use crate::product::models::ProjectRecord;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateProjectInput {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProjectStore {
    paths: ProductAppPaths,
}

impl ProjectStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn list(&self) -> Result<Vec<ProjectRecord>, ProductStoreError> {
        let projects_root = self.paths.projects_root();
        if !path_exists(&projects_root)? {
            return Ok(Vec::new());
        }

        let mut project_files = Vec::new();
        for entry in fs::read_dir(&projects_root).map_err(|error| {
            ProductStoreError::Io(format!("read {}: {error}", projects_root.display()))
        })? {
            let entry = entry.map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", projects_root.display()))
            })?;
            let project_path = entry.path().join("project.json");
            if path_exists(&project_path)? {
                project_files.push(project_path);
            }
        }
        project_files.sort();

        let mut projects = Vec::with_capacity(project_files.len());
        for project_file in project_files {
            let project: ProjectRecord = read_json(&project_file)?;
            LogicalCodebaseStore::new(self.paths.clone()).migrate_legacy(&project.id)?;
            projects.push(project);
        }
        Ok(projects)
    }

    pub fn get(&self, project_id: &str) -> Result<ProjectRecord, ProductStoreError> {
        validate_relative_id(project_id)?;
        let project_path = self.project_path(project_id);
        if !path_exists(&project_path)? {
            return Err(ProductStoreError::NotFound {
                kind: "project",
                id: project_id.to_string(),
            });
        }

        let project: ProjectRecord = read_json(&project_path)?;
        LogicalCodebaseStore::new(self.paths.clone()).migrate_legacy(project_id)?;
        Ok(project)
    }

    pub fn create(&self, input: CreateProjectInput) -> Result<ProjectRecord, ProductStoreError> {
        let existing_len = count_entries(&self.paths.projects_root())?;
        let id = next_sequential_id("project", existing_len);
        let now = Utc::now().to_rfc3339();
        let project = ProjectRecord {
            id: id.clone(),
            name: input.name,
            description: input.description,
            created_at: now.clone(),
            updated_at: now,
            last_opened_at: None,
        };

        write_json(&self.paths.project_root(&id).join("project.json"), &project)?;
        Ok(project)
    }

    pub fn open(&self, project_id: &str) -> Result<ProjectRecord, ProductStoreError> {
        let mut project = self.get(project_id)?;
        let now = Utc::now().to_rfc3339();
        project.updated_at = now.clone();
        project.last_opened_at = Some(now);

        write_json(&self.project_path(project_id), &project)?;
        write_json(&self.paths.last_project_path(), &project.id)?;
        Ok(project)
    }

    pub fn delete(&self, project_id: &str) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        let project_root = self.paths.project_root(project_id);
        if !path_exists(&project_root)? {
            return Err(ProductStoreError::NotFound {
                kind: "project",
                id: project_id.to_string(),
            });
        }
        fs::remove_dir_all(&project_root).map_err(|error| {
            ProductStoreError::Io(format!("remove {}: {error}", project_root.display()))
        })?;

        let last_project_path = self.paths.last_project_path();
        if path_exists(&last_project_path)?
            && read_json::<String>(&last_project_path)? == project_id
        {
            fs::remove_file(&last_project_path).map_err(|error| {
                ProductStoreError::Io(format!("remove {}: {error}", last_project_path.display()))
            })?;
        }
        Ok(())
    }

    fn project_path(&self, project_id: &str) -> std::path::PathBuf {
        self.paths.project_root(project_id).join("project.json")
    }
}

fn count_entries(path: &Path) -> Result<usize, ProductStoreError> {
    if !path_exists(path)? {
        return Ok(0);
    }

    fs::read_dir(path)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))?
        .try_fold(0usize, |count, entry| {
            entry.map(|_| count + 1).map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", path.display()))
            })
        })
}

fn path_exists(path: &Path) -> Result<bool, ProductStoreError> {
    path.try_exists()
        .map_err(|error| ProductStoreError::Io(format!("try_exists {}: {error}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::{CreateProjectInput, ProjectStore};
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::LogicalCodebaseStore;

    #[test]
    fn project_store_ignores_legacy_multi_repo_json_field() {
        let root = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let store = ProjectStore::new(paths.clone());
        let created = store
            .create(CreateProjectInput {
                name: "Aggregate".into(),
                description: None,
            })
            .unwrap();
        let stored =
            std::fs::read_to_string(paths.project_root(&created.id).join("project.json")).unwrap();
        assert!(!stored.contains("multi_repo"));

        let legacy_root = paths.logical_codebase_root("project_legacy");
        std::fs::create_dir_all(&legacy_root).unwrap();
        std::fs::write(
            paths.project_root("project_legacy").join("project.json"),
            r#"{"id":"project_legacy","name":"Legacy","description":null,"created_at":"2026-08-18T00:00:00Z","updated_at":"2026-08-18T00:00:00Z","last_opened_at":null,"multi_repo":true}"#,
        )
        .unwrap();
        std::fs::write(
            legacy_root.join("manifest.json"),
            r#"{"schema_version":1,"project_id":"project_legacy","logical_codebase_id":"018f0f8e-2c2d-7a10-8a11-111111111111","provider_context_root":"/workspace/legacy","layout":"common_non_git_parent","membership_revision":1,"member_ids":[],"created_at":"2026-08-18T00:00:00Z","updated_at":"2026-08-18T00:00:00Z"}"#,
        )
        .unwrap();

        assert_eq!(store.get("project_legacy").unwrap().name, "Legacy");
        let records = LogicalCodebaseStore::new(paths)
            .list("project_legacy")
            .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].name, "legacy");
    }
}
