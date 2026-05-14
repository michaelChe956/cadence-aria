use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::product::app_paths::ProductAppPaths;
use crate::product::id::{next_sequential_id, repo_hash_for_path};
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
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

    pub fn list(&self, project_id: &str) -> Result<Vec<RepositoryRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        let path = self.repos_path(project_id);
        if !path.exists() {
            return Ok(Vec::new());
        }

        read_json(&path)
    }

    pub fn create(
        &self,
        input: CreateRepositoryInput,
    ) -> Result<RepositoryRecord, ProductStoreError> {
        let project_id = input.project_id;
        let mut repositories = self.list(&project_id)?;
        let existing_len = repositories.len();
        let id = next_sequential_id("repository", existing_len);
        let now = Utc::now().to_rfc3339();
        let canonical_path = canonicalize_repo_path(&input.path)?;
        let repo_path_text = canonical_path.to_string_lossy();
        let repository = RepositoryRecord {
            id: id.clone(),
            project_id: project_id.clone(),
            name: input.name,
            repo_hash: repo_hash_for_path(repo_path_text.as_ref()),
            runtime_root: canonical_path.join(".aria/runtime"),
            path: canonical_path,
            default_policy_preset: input
                .default_policy_preset
                .unwrap_or_else(|| "manual-write".to_string()),
            default_provider_mode: input
                .default_provider_mode
                .unwrap_or_else(|| "fake".to_string()),
            created_at: now.clone(),
            updated_at: now,
        };

        repositories.push(repository.clone());
        write_json(&self.repos_path(&project_id), &repositories)?;
        Ok(repository)
    }

    pub fn find_by_path(
        &self,
        project_id: &str,
        path: &Path,
    ) -> Result<Option<RepositoryRecord>, ProductStoreError> {
        let canonical_path = canonicalize_repo_path(path)?;
        let canonical_text = canonical_path.to_string_lossy();
        let target_hash = repo_hash_for_path(canonical_text.as_ref());

        Ok(self.list(project_id)?.into_iter().find(|record| {
            if record.repo_hash == target_hash {
                return true;
            }

            fs::canonicalize(&record.path)
                .map(|record_path| record_path == canonical_path)
                .unwrap_or_else(|_| record.path.to_string_lossy() == canonical_text)
        }))
    }

    fn repos_path(&self, project_id: &str) -> PathBuf {
        self.paths.project_root(project_id).join("repos.json")
    }
}

fn canonicalize_repo_path(path: &Path) -> Result<PathBuf, ProductStoreError> {
    fs::canonicalize(path)
        .map_err(|error| ProductStoreError::Io(format!("canonicalize {}: {error}", path.display())))
}
