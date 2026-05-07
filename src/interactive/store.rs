use std::path::{Component, Path, PathBuf};

use serde::{Serialize, de::DeserializeOwned};

use crate::interactive::models::{InteractionTurn, NodeRun, TaskSession, WorkspaceProjection};
use crate::task_run::types::TaskRunError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractiveStore {
    workspace_root: PathBuf,
    task_id: String,
}

impl InteractiveStore {
    pub fn new(workspace_root: &Path, task_id: impl Into<String>) -> Self {
        Self {
            workspace_root: workspace_root.to_path_buf(),
            task_id: task_id.into(),
        }
    }

    pub fn task_root(&self) -> Result<PathBuf, TaskRunError> {
        validate_runtime_id(&self.task_id)?;
        Ok(self
            .workspace_root
            .join(".aria/runtime/tasks")
            .join(&self.task_id))
    }

    pub fn write_session(&self, session: &TaskSession) -> Result<PathBuf, TaskRunError> {
        validate_runtime_id(&session.session_id)?;
        let path = self.runtime_path(&format!("sessions/{}.json", session.session_id))?;
        write_json(&path, session)?;
        Ok(path)
    }

    pub fn read_session(&self, session_id: &str) -> Result<TaskSession, TaskRunError> {
        validate_runtime_id(session_id)?;
        read_json(&self.runtime_path(&format!("sessions/{session_id}.json"))?)
    }

    pub fn write_turn(&self, turn: &InteractionTurn) -> Result<PathBuf, TaskRunError> {
        validate_runtime_id(&turn.turn_id)?;
        let path = self.runtime_path(&format!("turns/{}.json", turn.turn_id))?;
        write_json(&path, turn)?;
        Ok(path)
    }

    pub fn read_turn(&self, turn_id: &str) -> Result<InteractionTurn, TaskRunError> {
        validate_runtime_id(turn_id)?;
        read_json(&self.runtime_path(&format!("turns/{turn_id}.json"))?)
    }

    pub fn write_node_run(&self, node_run: &NodeRun) -> Result<PathBuf, TaskRunError> {
        validate_runtime_id(&node_run.node_run_id)?;
        let path = self.runtime_path(&format!("node-runs/{}.json", node_run.node_run_id))?;
        write_json(&path, node_run)?;
        Ok(path)
    }

    pub fn read_node_run(&self, node_run_id: &str) -> Result<NodeRun, TaskRunError> {
        validate_runtime_id(node_run_id)?;
        read_json(&self.runtime_path(&format!("node-runs/{node_run_id}.json"))?)
    }

    pub fn write_projection(
        &self,
        projection: &WorkspaceProjection,
    ) -> Result<PathBuf, TaskRunError> {
        let path = self.runtime_path("projection.json")?;
        write_json(&path, projection)?;
        Ok(path)
    }

    pub fn read_projection(&self) -> Result<WorkspaceProjection, TaskRunError> {
        read_json(&self.runtime_path("projection.json")?)
    }

    fn runtime_path(&self, relative_path: &str) -> Result<PathBuf, TaskRunError> {
        validate_runtime_relative_path(Path::new(relative_path))?;
        Ok(self.task_root()?.join(relative_path))
    }
}

fn validate_runtime_id(value: &str) -> Result<(), TaskRunError> {
    if value.is_empty() || value.contains('/') || value.contains('\\') || value.contains("..") {
        return Err(TaskRunError::new(
            "interactive_store_invalid_id",
            format!("invalid runtime id: {value}"),
        ));
    }
    Ok(())
}

fn validate_runtime_relative_path(relative_path: &Path) -> Result<(), TaskRunError> {
    if relative_path.components().any(|component| {
        matches!(
            component,
            Component::Prefix(_) | Component::RootDir | Component::ParentDir
        )
    }) {
        return Err(TaskRunError::new(
            "interactive_store_path_escape",
            format!(
                "interactive store path escapes task root: {}",
                relative_path.display()
            ),
        ));
    }
    Ok(())
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), TaskRunError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            TaskRunError::new(
                "interactive_store_io",
                format!("create {}: {error}", parent.display()),
            )
        })?;
    }
    let file = std::fs::File::create(path).map_err(|error| {
        TaskRunError::new(
            "interactive_store_io",
            format!("create {}: {error}", path.display()),
        )
    })?;
    serde_json::to_writer_pretty(file, value)
        .map_err(|error| TaskRunError::new("interactive_store_serialize", error.to_string()))
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, TaskRunError> {
    let file = std::fs::File::open(path).map_err(|error| {
        TaskRunError::new(
            "interactive_store_io",
            format!("open {}: {error}", path.display()),
        )
    })?;
    serde_json::from_reader(file)
        .map_err(|error| TaskRunError::new("interactive_store_deserialize", error.to_string()))
}
