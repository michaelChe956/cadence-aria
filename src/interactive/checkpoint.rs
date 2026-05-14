use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

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
pub struct RollbackPreviewRequest {
    pub checkpoint_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackPreview {
    pub checkpoint_id: String,
    pub git_head: Option<String>,
    pub dirty: bool,
    pub turns_to_drop: usize,
    pub node_runs_to_drop: usize,
    pub provider_runs_to_drop: usize,
    pub artifacts_to_drop: usize,
    pub files_may_change: Vec<String>,
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

    pub fn preview_rollback(
        &self,
        request: RollbackPreviewRequest,
    ) -> Result<RollbackPreview, TaskRunError> {
        let checkpoint = self.read_checkpoint(&request.checkpoint_id)?;
        let task_root = self.task_root();
        Ok(RollbackPreview {
            checkpoint_id: checkpoint.checkpoint_id,
            git_head: checkpoint.git_head,
            dirty: self.worktree_dirty()?,
            turns_to_drop: count_turns_to_drop(
                &task_root.join("turns"),
                checkpoint.turn_id.as_deref(),
            )?,
            node_runs_to_drop: count_json_files_after_boundary(
                &task_root.join("node-runs"),
                checkpoint.node_run_boundary,
            )?,
            provider_runs_to_drop: count_provider_runs_after_boundary(
                &task_root.join("provider-runs"),
                checkpoint.provider_run_boundary,
            )?,
            artifacts_to_drop: count_runtime_artifacts_after_boundary(
                &task_root,
                checkpoint.artifact_boundary,
            )?,
            files_may_change: self.changed_files()?,
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
        restore_snapshot(
            &task_root,
            &checkpoint.state_snapshot_ref,
            &task_root.join("state.json"),
        )?;
        restore_snapshot(
            &task_root,
            &checkpoint.projection_snapshot_ref,
            &task_root.join("projection.json"),
        )?;
        mark_turns_dropped(&task_root.join("turns"), checkpoint.turn_id.as_deref())?;
        mark_json_files_dropped_after_boundary(
            &task_root.join("node-runs"),
            checkpoint.node_run_boundary,
        )?;
        mark_provider_runs_dropped_after_boundary(
            &task_root.join("provider-runs"),
            checkpoint.provider_run_boundary,
        )?;
        mark_runtime_artifacts_dropped_after_boundary(&task_root, checkpoint.artifact_boundary)?;
        Ok(())
    }

    pub fn ensure_reset_target_is_not_repo_root(
        &self,
        reset_target: &Path,
    ) -> Result<(), TaskRunError> {
        let repo_root = self.workspace_root.canonicalize().map_err(|error| {
            TaskRunError::new("checkpoint_io", format!("canonicalize workspace: {error}"))
        })?;
        let reset_target = reset_target.canonicalize().map_err(|error| {
            TaskRunError::new(
                "checkpoint_io",
                format!("canonicalize reset target: {error}"),
            )
        })?;
        if reset_target == repo_root {
            return Err(TaskRunError::new(
                "checkpoint_refuse_repo_root_reset",
                "development rollback must target a work item worktree, not the repository root",
            ));
        }
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

    fn changed_files(&self) -> Result<Vec<String>, TaskRunError> {
        let output = self.git(&["status", "--porcelain"])?;
        Ok(output
            .lines()
            .filter_map(|line| line.get(3..))
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(str::to_string)
            .collect())
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

fn restore_snapshot(
    task_root: &Path,
    snapshot_ref: &str,
    active_path: &Path,
) -> Result<(), TaskRunError> {
    let snapshot_path = snapshot_path(task_root, snapshot_ref)?;
    if let Some(parent) = active_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            TaskRunError::new(
                "checkpoint_io",
                format!("create {}: {error}", parent.display()),
            )
        })?;
    }
    fs::copy(&snapshot_path, active_path).map_err(|error| {
        TaskRunError::new(
            "checkpoint_io",
            format!(
                "restore {} to {}: {error}",
                snapshot_path.display(),
                active_path.display()
            ),
        )
    })?;
    Ok(())
}

fn snapshot_path(task_root: &Path, snapshot_ref: &str) -> Result<PathBuf, TaskRunError> {
    let snapshot_ref_path = Path::new(snapshot_ref);
    if snapshot_ref_path.is_absolute()
        || snapshot_ref.contains('/')
        || snapshot_ref.contains('\\')
        || snapshot_ref.contains("..")
        || snapshot_ref.is_empty()
    {
        return Err(TaskRunError::new(
            "checkpoint_invalid_snapshot_ref",
            format!("invalid checkpoint snapshot ref: {snapshot_ref}"),
        ));
    }
    Ok(task_root.join("checkpoints").join(snapshot_ref))
}

fn mark_turns_dropped(root: &Path, turn_id: Option<&str>) -> Result<(), TaskRunError> {
    let files = json_files(root)?;
    let start = turn_id
        .and_then(|turn_id| {
            files.iter().position(|path| {
                path.file_stem()
                    .and_then(|name| name.to_str())
                    .is_some_and(|stem| stem == turn_id)
            })
        })
        .unwrap_or(0);
    mark_json_paths_dropped(files.into_iter().skip(start))
}

fn count_turns_to_drop(root: &Path, turn_id: Option<&str>) -> Result<usize, TaskRunError> {
    let files = json_files(root)?;
    let start = turn_id
        .and_then(|turn_id| {
            files.iter().position(|path| {
                path.file_stem()
                    .and_then(|name| name.to_str())
                    .is_some_and(|stem| stem == turn_id)
            })
        })
        .unwrap_or(0);
    Ok(files.len().saturating_sub(start))
}

fn mark_json_files_dropped_after_boundary(
    root: &Path,
    boundary: usize,
) -> Result<(), TaskRunError> {
    let files = json_files(root)?;
    mark_json_paths_dropped(files.into_iter().skip(boundary))
}

fn count_json_files_after_boundary(root: &Path, boundary: usize) -> Result<usize, TaskRunError> {
    let files = json_files(root)?;
    Ok(files.len().saturating_sub(boundary))
}

fn mark_runtime_artifacts_dropped_after_boundary(
    task_root: &Path,
    boundary: usize,
) -> Result<(), TaskRunError> {
    let files = runtime_artifact_files(task_root)?;
    mark_json_paths_dropped(files.into_iter().skip(boundary))
}

fn count_runtime_artifacts_after_boundary(
    task_root: &Path,
    boundary: usize,
) -> Result<usize, TaskRunError> {
    let files = runtime_artifact_files(task_root)?;
    Ok(files.len().saturating_sub(boundary))
}

fn runtime_artifact_files(task_root: &Path) -> Result<Vec<PathBuf>, TaskRunError> {
    let mut files = json_files(&task_root.join("artifacts"))?;
    files.extend(json_files(&task_root.join("reports"))?);
    sort_paths_by_modified_then_path(&mut files);
    Ok(files)
}

fn mark_provider_runs_dropped_after_boundary(
    root: &Path,
    boundary: usize,
) -> Result<(), TaskRunError> {
    let files = provider_run_files(root)?;
    mark_json_paths_dropped(files.into_iter().skip(boundary))
}

fn count_provider_runs_after_boundary(root: &Path, boundary: usize) -> Result<usize, TaskRunError> {
    let files = provider_run_files(root)?;
    Ok(files.len().saturating_sub(boundary))
}

fn provider_run_files(root: &Path) -> Result<Vec<PathBuf>, TaskRunError> {
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    for entry in fs::read_dir(root).map_err(|error| {
        TaskRunError::new("checkpoint_io", format!("read {}: {error}", root.display()))
    })? {
        let path = entry
            .map_err(|error| TaskRunError::new("checkpoint_io", error.to_string()))?
            .path();
        let run_path = path.join("run.json");
        if run_path.exists() {
            files.push(run_path);
        }
    }
    sort_paths_by_modified_then_path(&mut files);
    Ok(files)
}

fn json_files(root: &Path) -> Result<Vec<PathBuf>, TaskRunError> {
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    collect_json_files(root, &mut files)?;
    sort_paths_by_modified_then_path(&mut files);
    Ok(files)
}

fn sort_paths_by_modified_then_path(files: &mut [PathBuf]) {
    files.sort_by(|left, right| {
        modified_time(left)
            .cmp(&modified_time(right))
            .then_with(|| left.cmp(right))
    });
}

fn modified_time(path: &Path) -> SystemTime {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .unwrap_or(UNIX_EPOCH)
}

fn collect_json_files(root: &Path, files: &mut Vec<PathBuf>) -> Result<(), TaskRunError> {
    for entry in fs::read_dir(root).map_err(|error| {
        TaskRunError::new("checkpoint_io", format!("read {}: {error}", root.display()))
    })? {
        let path = entry
            .map_err(|error| TaskRunError::new("checkpoint_io", error.to_string()))?
            .path();
        if path.is_dir() {
            collect_json_files(&path, files)?;
        } else if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            files.push(path);
        }
    }
    Ok(())
}

fn mark_json_paths_dropped(paths: impl IntoIterator<Item = PathBuf>) -> Result<(), TaskRunError> {
    for path in paths {
        mark_json_file_dropped(&path)?;
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
