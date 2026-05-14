use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::product::app_paths::ProductAppPaths;
use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, validate_relative_id, write_json};
use crate::product::models::{IssueRuntimeBindingRecord, RuntimeBindingStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateRuntimeBindingInput {
    pub project_id: String,
    pub issue_id: String,
    pub repo_id: String,
    pub change_id: String,
    pub task_id: Option<String>,
    pub session_id: Option<String>,
    pub runtime_root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct RuntimeBindingStore {
    paths: ProductAppPaths,
}

impl RuntimeBindingStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn create(
        &self,
        input: CreateRuntimeBindingInput,
    ) -> Result<IssueRuntimeBindingRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.repo_id)?;
        let bindings_root = self
            .paths
            .issue_root(&input.project_id, &input.issue_id)
            .join("bindings");
        let existing_len = count_entries(&bindings_root)?;
        let id = next_sequential_id("binding", existing_len);
        let now = Utc::now().to_rfc3339();
        let task_root = input
            .task_id
            .as_ref()
            .map(|task_id| input.runtime_root.join("tasks").join(task_id));
        let binding = IssueRuntimeBindingRecord {
            id: id.clone(),
            issue_id: input.issue_id,
            repo_id: input.repo_id,
            change_id: input.change_id,
            task_id: input.task_id,
            session_id: input.session_id,
            runtime_root: input.runtime_root,
            task_root,
            status: RuntimeBindingStatus::Created,
            created_at: now.clone(),
            updated_at: now,
        };

        write_json(&bindings_root.join(format!("{id}.json")), &binding)?;
        Ok(binding)
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
