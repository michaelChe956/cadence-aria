use std::path::{Component, Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::task_run::types::TaskRunError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspacePreflight {
    pub workspace_root: PathBuf,
    pub openspec_config: PathBuf,
}

pub fn preflight_workspace(workspace_root: &Path) -> Result<WorkspacePreflight, TaskRunError> {
    if !workspace_root.exists() {
        return Err(TaskRunError::new(
            "workspace_missing",
            format!("workspace does not exist: {}", workspace_root.display()),
        ));
    }
    if !workspace_root.is_dir() {
        return Err(TaskRunError::new(
            "workspace_not_directory",
            format!("workspace is not a directory: {}", workspace_root.display()),
        ));
    }

    let output = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(workspace_root)
        .output()
        .map_err(|error| {
            TaskRunError::new("git_command_failed", format!("run git rev-parse: {error}"))
        })?;
    if !output.status.success() || String::from_utf8_lossy(&output.stdout).trim() != "true" {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TaskRunError::new(
            "workspace_not_git_worktree",
            format!(
                "workspace is not a git worktree: {}; status={}; stderr={}",
                workspace_root.display(),
                output.status,
                stderr.trim()
            ),
        ));
    }

    let openspec_config = workspace_root.join("openspec/config.yaml");
    if !openspec_config.is_file() {
        return Err(TaskRunError::new(
            "openspec_config_missing",
            format!("missing {}", openspec_config.display()),
        ));
    }

    Ok(WorkspacePreflight {
        workspace_root: workspace_root.to_path_buf(),
        openspec_config,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRunStore {
    workspace_root: PathBuf,
    task_id: String,
}

impl TaskRunStore {
    pub fn new(workspace_root: &Path, task_id: impl Into<String>) -> Self {
        Self {
            workspace_root: workspace_root.to_path_buf(),
            task_id: task_id.into(),
        }
    }

    pub fn task_root(&self) -> PathBuf {
        self.workspace_root
            .join(".aria/runtime/tasks")
            .join(&self.task_id)
    }

    pub fn state_path(&self) -> PathBuf {
        self.task_root().join("state.json")
    }

    pub fn write_task_state(&self, value: &serde_json::Value) -> Result<PathBuf, TaskRunError> {
        write_json_file(&self.state_path(), value)?;
        Ok(self.state_path())
    }

    pub fn write_json_report<T: Serialize>(
        &self,
        name: &str,
        value: &T,
    ) -> Result<PathBuf, TaskRunError> {
        validate_runtime_relative_path(name)?;
        let path = self.task_root().join("reports").join(name);
        write_json_file(&path, value)?;
        Ok(path)
    }

    pub fn write_json_artifact<T: Serialize>(
        &self,
        relative_path: &str,
        value: &T,
    ) -> Result<PathBuf, TaskRunError> {
        validate_runtime_relative_path(relative_path)?;
        let path = self.task_root().join(relative_path);
        write_json_file(&path, value)?;
        Ok(path)
    }
}

fn validate_runtime_relative_path(relative_path: &str) -> Result<(), TaskRunError> {
    let path = Path::new(relative_path);
    if path.components().any(|component| {
        matches!(
            component,
            Component::Prefix(_) | Component::RootDir | Component::ParentDir
        )
    }) {
        return Err(TaskRunError::new(
            "runtime_store_path_escape",
            format!("runtime store path escapes task root: {relative_path}"),
        ));
    }
    Ok(())
}

fn write_json_file<T: Serialize>(path: &Path, value: &T) -> Result<(), TaskRunError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            TaskRunError::new(
                "runtime_store_io",
                format!("create {}: {error}", parent.display()),
            )
        })?;
    }
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| TaskRunError::new("runtime_store_serialize", error.to_string()))?;
    std::fs::write(path, bytes).map_err(|error| {
        TaskRunError::new(
            "runtime_store_io",
            format!("write {}: {error}", path.display()),
        )
    })
}
