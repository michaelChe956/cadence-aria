use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use serde_json::Value;

use crate::interactive::models::RuntimeCheckpoint;
use crate::task_run::types::TaskRunError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackRequest {
    pub checkpoint_id: String,
    pub force_when_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointService {
    workspace_root: PathBuf,
    task_id: String,
}

impl CheckpointService {
    pub fn new(workspace_root: &Path, task_id: impl Into<String>) -> Self {
        Self {
            workspace_root: workspace_root.to_path_buf(),
            task_id: task_id.into(),
        }
    }

    pub fn write_checkpoint(
        &self,
        checkpoint: &RuntimeCheckpoint,
    ) -> Result<PathBuf, TaskRunError> {
        validate_checkpoint_id(&checkpoint.checkpoint_id)?;
        let path = self.checkpoint_path(&checkpoint.checkpoint_id)?;
        write_json(&path, checkpoint)?;
        Ok(path)
    }

    pub fn read_checkpoint(&self, checkpoint_id: &str) -> Result<RuntimeCheckpoint, TaskRunError> {
        validate_checkpoint_id(checkpoint_id)?;
        let path = self.checkpoint_path(checkpoint_id)?;
        let file = fs::File::open(&path).map_err(|error| {
            TaskRunError::new("checkpoint_io", format!("open {}: {error}", path.display()))
        })?;
        serde_json::from_reader(file).map_err(|error| {
            TaskRunError::new(
                "checkpoint_json",
                format!("parse {}: {error}", path.display()),
            )
        })
    }

    pub fn rollback(&self, request: RollbackRequest) -> Result<(), TaskRunError> {
        let checkpoint = self.read_checkpoint(&request.checkpoint_id)?;
        if !request.force_when_dirty && self.worktree_dirty()? {
            return Err(TaskRunError::new(
                "checkpoint_unsafe_dirty_worktree",
                "worktree has uncommitted changes; use force_when_dirty to rollback",
            ));
        }

        if let Some(git_head) = checkpoint.git_head.as_deref() {
            self.git(&["reset", "--hard", git_head])?;
        }

        let task_root = self.task_root();
        mark_json_files_dropped(&task_root.join("turns"))?;
        mark_json_files_dropped(&task_root.join("node-runs"))?;
        mark_provider_runs_dropped(&task_root.join("provider-runs"))?;
        Ok(())
    }

    fn checkpoint_path(&self, checkpoint_id: &str) -> Result<PathBuf, TaskRunError> {
        validate_checkpoint_id(checkpoint_id)?;
        Ok(self
            .task_root()
            .join("checkpoints")
            .join(format!("{checkpoint_id}.json")))
    }

    fn task_root(&self) -> PathBuf {
        self.workspace_root
            .join(".aria/runtime/tasks")
            .join(&self.task_id)
    }

    fn worktree_dirty(&self) -> Result<bool, TaskRunError> {
        let output = self.git(&["status", "--porcelain"])?;
        Ok(!output.trim().is_empty())
    }

    fn git(&self, args: &[&str]) -> Result<String, TaskRunError> {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.workspace_root)
            .output()
            .map_err(|error| {
                TaskRunError::new("git_command_failed", format!("run git {args:?}: {error}"))
            })?;
        if !output.status.success() {
            return Err(TaskRunError::new(
                "git_command_failed",
                format!(
                    "git {:?} failed stdout={} stderr={}",
                    args,
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                ),
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

fn validate_checkpoint_id(checkpoint_id: &str) -> Result<(), TaskRunError> {
    if checkpoint_id.is_empty()
        || checkpoint_id.contains('/')
        || checkpoint_id.contains('\\')
        || checkpoint_id.contains("..")
    {
        return Err(TaskRunError::new(
            "checkpoint_invalid_id",
            format!("invalid checkpoint id: {checkpoint_id}"),
        ));
    }
    Ok(())
}

fn mark_json_files_dropped(root: &Path) -> Result<(), TaskRunError> {
    if !root.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(root).map_err(|error| {
        TaskRunError::new("checkpoint_io", format!("read {}: {error}", root.display()))
    })? {
        let path = entry
            .map_err(|error| TaskRunError::new("checkpoint_io", error.to_string()))?
            .path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            mark_json_file_dropped(&path)?;
        }
    }
    Ok(())
}

fn mark_provider_runs_dropped(root: &Path) -> Result<(), TaskRunError> {
    if !root.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(root).map_err(|error| {
        TaskRunError::new("checkpoint_io", format!("read {}: {error}", root.display()))
    })? {
        let path = entry
            .map_err(|error| TaskRunError::new("checkpoint_io", error.to_string()))?
            .path()
            .join("run.json");
        if path.exists() {
            mark_json_file_dropped(&path)?;
        }
    }
    Ok(())
}

fn mark_json_file_dropped(path: &Path) -> Result<(), TaskRunError> {
    let file = fs::File::open(path).map_err(|error| {
        TaskRunError::new("checkpoint_io", format!("open {}: {error}", path.display()))
    })?;
    let mut value: Value = serde_json::from_reader(file).map_err(|error| {
        TaskRunError::new(
            "checkpoint_json",
            format!("parse {}: {error}", path.display()),
        )
    })?;
    value["dropped"] = Value::Bool(true);
    write_json(path, &value)
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), TaskRunError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            TaskRunError::new(
                "checkpoint_io",
                format!("create {}: {error}", parent.display()),
            )
        })?;
    }

    let file = fs::File::create(path).map_err(|error| {
        TaskRunError::new(
            "checkpoint_io",
            format!("create {}: {error}", path.display()),
        )
    })?;
    serde_json::to_writer_pretty(file, value).map_err(|error| {
        TaskRunError::new(
            "checkpoint_json",
            format!("write {}: {error}", path.display()),
        )
    })
}
