use std::fs;
use std::path::Path;

use chrono::Utc;

use crate::product::app_paths::ProductAppPaths;
use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, write_json};
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
}

fn count_entries(path: &Path) -> Result<usize, ProductStoreError> {
    if !path.exists() {
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
