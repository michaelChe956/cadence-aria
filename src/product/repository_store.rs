use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::product::app_paths::ProductAppPaths;
use crate::product::id::{next_sequential_id, repo_hash_for_path};
use crate::product::json_store::{ProductStoreError, validate_relative_id, write_json};
use crate::product::models::RepositoryRecord;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateRepositoryInput {
    pub project_id: String,
    pub name: String,
    pub path: PathBuf,
    pub default_policy_preset: Option<String>,
    pub default_provider_mode: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RepositoryStore {
    paths: ProductAppPaths,
}

impl RepositoryStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn create(
        &self,
        input: CreateRepositoryInput,
    ) -> Result<RepositoryRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        let repositories_root = self
            .paths
            .project_root(&input.project_id)
            .join("repositories");
        let existing_len = count_entries(&repositories_root)?;
        let id = next_sequential_id("repository", existing_len);
        let now = Utc::now().to_rfc3339();
        let repo_path_text = input.path.to_string_lossy();
        let repository = RepositoryRecord {
            id: id.clone(),
            project_id: input.project_id,
            name: input.name,
            repo_hash: repo_hash_for_path(repo_path_text.as_ref()),
            runtime_root: input.path.join(".aria/runtime"),
            path: input.path,
            default_policy_preset: input
                .default_policy_preset
                .unwrap_or_else(|| "manual-write".to_string()),
            default_provider_mode: input
                .default_provider_mode
                .unwrap_or_else(|| "fake".to_string()),
            created_at: now.clone(),
            updated_at: now,
        };

        write_json(&repositories_root.join(format!("{id}.json")), &repository)?;
        Ok(repository)
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
