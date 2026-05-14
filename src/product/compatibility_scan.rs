use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::product::app_paths::ProductAppPaths;
use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
use crate::product::json_store::{ProductStoreError, read_json};
use crate::product::models::{IssueRecord, ProjectRecord, RepositoryRecord};
use crate::product::project_store::{CreateProjectInput, ProjectStore};
use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};
use crate::product::runtime_binding_store::{CreateRuntimeBindingInput, RuntimeBindingStore};

#[derive(Debug, Clone)]
pub struct CompatibilityScanInput {
    pub app_paths: ProductAppPaths,
    pub repo_path: PathBuf,
    pub project_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibilityScanSummary {
    pub projects_created: usize,
    pub repositories_created: usize,
    pub issues_created: usize,
    pub bindings_created: usize,
}

pub fn rebuild_index_from_runtime(
    input: CompatibilityScanInput,
) -> Result<CompatibilityScanSummary, ProductStoreError> {
    let tasks_root = input.repo_path.join(".aria/runtime/tasks");
    if !tasks_root.exists() {
        return Ok(empty_summary());
    }

    let runtime_tasks = read_runtime_tasks(&tasks_root)?;
    if runtime_tasks.is_empty() {
        return Ok(empty_summary());
    }

    let mut summary = empty_summary();
    let project = find_or_create_project(&input.app_paths, &input.project_name, &mut summary)?;
    let repository = find_or_create_repository(
        &input.app_paths,
        &project.id,
        &input.repo_path,
        &mut summary,
    )?;
    let issue_store = IssueStore::new(input.app_paths.clone());
    let binding_store = RuntimeBindingStore::new(input.app_paths.clone());

    for runtime_task in runtime_tasks {
        let issue = match find_issue_by_change_id(
            &input.app_paths,
            &project.id,
            &repository.id,
            &runtime_task.change_id,
        )? {
            Some(issue) => issue,
            None => {
                summary.issues_created += 1;
                issue_store.create(CreateProductIssueInput {
                    project_id: project.id.clone(),
                    repo_id: repository.id.clone(),
                    title: runtime_task.title.clone(),
                    description: None,
                    change_id: Some(runtime_task.change_id.clone()),
                })?
            }
        };

        let existing_binding = binding_store.find_by_repo_and_task(
            &project.id,
            &issue.id,
            &repository.id,
            &runtime_task.task_id,
        )?;
        if existing_binding.is_some() {
            continue;
        }

        binding_store.create(CreateRuntimeBindingInput {
            project_id: project.id.clone(),
            issue_id: issue.id,
            repo_id: repository.id.clone(),
            change_id: issue.change_id,
            task_id: Some(runtime_task.task_id),
            session_id: None,
            runtime_root: repository.runtime_root.clone(),
        })?;
        summary.bindings_created += 1;
    }

    Ok(summary)
}

#[derive(Debug, Clone)]
struct RuntimeTaskSnapshot {
    task_id: String,
    change_id: String,
    title: String,
}

fn empty_summary() -> CompatibilityScanSummary {
    CompatibilityScanSummary {
        projects_created: 0,
        repositories_created: 0,
        issues_created: 0,
        bindings_created: 0,
    }
}

fn find_or_create_project(
    app_paths: &ProductAppPaths,
    project_name: &str,
    summary: &mut CompatibilityScanSummary,
) -> Result<ProjectRecord, ProductStoreError> {
    if let Some(project) = find_project(app_paths, project_name)? {
        return Ok(project);
    }

    summary.projects_created += 1;
    ProjectStore::new(app_paths.clone()).create(CreateProjectInput {
        name: project_name.to_string(),
        description: None,
    })
}

fn find_project(
    app_paths: &ProductAppPaths,
    project_name: &str,
) -> Result<Option<ProjectRecord>, ProductStoreError> {
    let projects = list_projects(app_paths)?;
    if let Some(project) = projects
        .iter()
        .find(|project| project.name == project_name)
        .cloned()
    {
        return Ok(Some(project));
    }

    Ok(projects.into_iter().next())
}

fn list_projects(app_paths: &ProductAppPaths) -> Result<Vec<ProjectRecord>, ProductStoreError> {
    let projects_root = app_paths.projects_root();
    if !projects_root.exists() {
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
        if project_path.exists() {
            project_files.push(project_path);
        }
    }
    project_files.sort();

    let mut projects = Vec::with_capacity(project_files.len());
    for project_file in project_files {
        projects.push(read_json(&project_file)?);
    }
    Ok(projects)
}

fn find_or_create_repository(
    app_paths: &ProductAppPaths,
    project_id: &str,
    repo_path: &Path,
    summary: &mut CompatibilityScanSummary,
) -> Result<RepositoryRecord, ProductStoreError> {
    let repository_store = RepositoryStore::new(app_paths.clone());
    if let Some(repository) = repository_store.find_by_path(project_id, repo_path)? {
        return Ok(repository);
    }

    let canonical_path = canonicalize_path(repo_path)?;
    let name = canonical_path
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "repository".to_string());

    summary.repositories_created += 1;
    repository_store.create(CreateRepositoryInput {
        project_id: project_id.to_string(),
        name,
        path: canonical_path,
        default_policy_preset: None,
        default_provider_mode: None,
    })
}

fn find_issue_by_change_id(
    app_paths: &ProductAppPaths,
    project_id: &str,
    repo_id: &str,
    change_id: &str,
) -> Result<Option<IssueRecord>, ProductStoreError> {
    Ok(list_issues(app_paths, project_id)?
        .into_iter()
        .find(|issue| issue.repo_id == repo_id && issue.change_id == change_id))
}

fn list_issues(
    app_paths: &ProductAppPaths,
    project_id: &str,
) -> Result<Vec<IssueRecord>, ProductStoreError> {
    let issues_root = app_paths.project_root(project_id).join("issues");
    if !issues_root.exists() {
        return Ok(Vec::new());
    }

    let mut issue_files = Vec::new();
    for entry in fs::read_dir(&issues_root).map_err(|error| {
        ProductStoreError::Io(format!("read {}: {error}", issues_root.display()))
    })? {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", issues_root.display()))
        })?;
        let issue_path = entry.path().join("issue.json");
        if issue_path.exists() {
            issue_files.push(issue_path);
        }
    }
    issue_files.sort();

    let mut issues = Vec::with_capacity(issue_files.len());
    for issue_file in issue_files {
        issues.push(read_json(&issue_file)?);
    }
    Ok(issues)
}

fn read_runtime_tasks(tasks_root: &Path) -> Result<Vec<RuntimeTaskSnapshot>, ProductStoreError> {
    let mut task_dirs = Vec::new();
    for entry in fs::read_dir(tasks_root)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", tasks_root.display())))?
    {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", tasks_root.display()))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            ProductStoreError::Io(format!(
                "read {} file type: {error}",
                entry.path().display()
            ))
        })?;
        if file_type.is_dir() {
            task_dirs.push(entry.path());
        }
    }
    task_dirs.sort();

    let mut runtime_tasks = Vec::new();
    for task_dir in task_dirs {
        let state_path = task_dir.join("state.json");
        if !state_path.exists() {
            continue;
        }

        let state: Value = read_json(&state_path)?;
        let fallback_task_id = task_dir
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| "task".to_string());
        let task_id = string_field(&state, "task_id").unwrap_or_else(|| fallback_task_id.clone());
        let change_id = string_field(&state, "change_id").unwrap_or_else(|| task_id.clone());
        let title = string_field(&state, "request_text").unwrap_or_else(|| change_id.clone());

        runtime_tasks.push(RuntimeTaskSnapshot {
            task_id,
            change_id,
            title,
        });
    }

    Ok(runtime_tasks)
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn canonicalize_path(path: &Path) -> Result<PathBuf, ProductStoreError> {
    fs::canonicalize(path)
        .map_err(|error| ProductStoreError::Io(format!("canonicalize {}: {error}", path.display())))
}
