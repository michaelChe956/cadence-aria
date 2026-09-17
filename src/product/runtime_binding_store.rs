use std::fs;
use std::path::PathBuf;

use chrono::Utc;

use crate::product::app_paths::ProductAppPaths;
use crate::product::id::next_sequential_id_in_directory;
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

        let mut entries = Vec::new();
        for entry in fs::read_dir(&path)
            .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))?
        {
            let entry = entry.map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", path.display()))
            })?;
            let entry_path = entry.path();
            if entry_path.extension().and_then(|value| value.to_str()) == Some("json") {
                entries.push(entry_path);
            }
        }
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
        let bindings_root = self.bindings_root(&project_id, &issue_id);
        let id = next_sequential_id_in_directory("binding", &bindings_root).map_err(|error| {
            ProductStoreError::Io(format!("read {}: {error}", bindings_root.display()))
        })?;
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
            logical_repository_id: None,
            checkout_id: None,
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
    ) -> Result<Option<IssueRuntimeBindingRecord>, ProductStoreError> {
        validate_relative_id(repo_id)?;
        Ok(self
            .list(project_id, issue_id)?
            .into_iter()
            .find(|binding| {
                binding.repo_id == repo_id && binding.task_id.as_deref() == Some(task_id)
            }))
    }

    fn bindings_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths.issue_root(project_id, issue_id).join("bindings")
    }

    fn binding_path(&self, project_id: &str, issue_id: &str, binding_id: &str) -> PathBuf {
        self.bindings_root(project_id, issue_id)
            .join(format!("{binding_id}.json"))
    }
}

#[cfg(test)]
mod tests {
    use super::{CreateRuntimeBindingInput, RuntimeBindingStore};
    use crate::product::app_paths::ProductAppPaths;

    fn input(project_id: &str, issue_id: &str, repo_id: &str) -> CreateRuntimeBindingInput {
        CreateRuntimeBindingInput {
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            repo_id: repo_id.to_string(),
            change_id: format!("change-{repo_id}"),
            task_id: None,
            session_id: None,
            runtime_root: std::path::PathBuf::from("/tmp/runtime"),
        }
    }

    #[test]
    fn create_after_deleting_middle_binding_uses_id_above_existing_maximum() {
        let root = tempfile::tempdir().unwrap();
        let store = RuntimeBindingStore::new(ProductAppPaths::new(root.path().join(".aria")));
        let _first = store.create(input("project_0001", "issue_0001", "repository_0001")).unwrap();
        let middle = store.create(input("project_0001", "issue_0001", "repository_0002")).unwrap();
        let _last = store.create(input("project_0001", "issue_0001", "repository_0003")).unwrap();
        std::fs::remove_file(
            root.path()
                .join(".aria/projects/project_0001/issues/issue_0001/bindings")
                .join(format!("{}.json", middle.id)),
        )
        .unwrap();

        let replacement = store.create(input("project_0001", "issue_0001", "repository_0004")).unwrap();

        assert_eq!(replacement.id, "binding_0004");
    }
}
