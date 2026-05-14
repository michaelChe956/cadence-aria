use std::fs;
use std::path::PathBuf;

use chrono::Utc;

use crate::product::app_paths::ProductAppPaths;
use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
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

    pub fn list(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<IssueRuntimeBindingRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let path = self.bindings_root(project_id, issue_id);
        if !path.exists() {
            return Ok(Vec::new());
        }

        let mut entries = fs::read_dir(&path)
            .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
            .collect::<Vec<_>>();
        entries.sort();

        let mut bindings = Vec::with_capacity(entries.len());
        for entry in entries {
            bindings.push(read_json(&entry)?);
        }
        Ok(bindings)
    }

    pub fn create(
        &self,
        input: CreateRuntimeBindingInput,
    ) -> Result<IssueRuntimeBindingRecord, ProductStoreError> {
        validate_relative_id(&input.repo_id)?;
        let project_id = input.project_id;
        let issue_id = input.issue_id;
        let bindings = self.list(&project_id, &issue_id)?;
        let existing_len = bindings.len();
        let id = next_sequential_id("binding", existing_len);
        let now = Utc::now().to_rfc3339();
        let task_root = input
            .task_id
            .as_ref()
            .map(|task_id| input.runtime_root.join("tasks").join(task_id));
        let binding = IssueRuntimeBindingRecord {
            id: id.clone(),
            issue_id: issue_id.clone(),
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

        write_json(&self.binding_path(&project_id, &issue_id, &id), &binding)?;
        Ok(binding)
    }

    pub fn find_by_repo_and_task(
        &self,
        project_id: &str,
        issue_id: &str,
        repo_id: &str,
        task_id: &str,
    ) -> Option<IssueRuntimeBindingRecord> {
        validate_relative_id(repo_id).ok()?;
        self.list(project_id, issue_id)
            .ok()?
            .into_iter()
            .find(|binding| {
                binding.repo_id == repo_id && binding.task_id.as_deref() == Some(task_id)
            })
    }

    fn bindings_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths.issue_root(project_id, issue_id).join("bindings")
    }

    fn binding_path(&self, project_id: &str, issue_id: &str, binding_id: &str) -> PathBuf {
        self.bindings_root(project_id, issue_id)
            .join(format!("{binding_id}.json"))
    }
}
