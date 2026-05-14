use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Value, json};

use crate::task_run::types::TaskRunError;

#[derive(Debug, Clone)]
pub struct WebRuntimeStore {
    task_root: PathBuf,
}

impl WebRuntimeStore {
    pub fn new(workspace_root: &Path, task_id: &str) -> Self {
        Self {
            task_root: workspace_root.join(".aria/runtime/tasks").join(task_id),
        }
    }

    pub fn task_root(&self) -> &Path {
        &self.task_root
    }

    pub fn write_json<T: Serialize>(
        &self,
        relative: &str,
        value: &T,
    ) -> Result<PathBuf, TaskRunError> {
        let path = self.task_root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        let file = fs::File::create(&path).map_err(io_error)?;
        serde_json::to_writer_pretty(file, value)
            .map_err(|error| TaskRunError::new("web_runtime_json", error.to_string()))?;
        Ok(path)
    }

    pub fn append_event(
        &self,
        event_kind: &str,
        node_id: &str,
        details: Value,
    ) -> Result<(), TaskRunError> {
        self.append_jsonl(
            "logs/node-events.jsonl",
            json!({
                "event_kind": event_kind,
                "task_id": self.task_root.file_name().and_then(|name| name.to_str()),
                "node_id": node_id,
                "status": details.get("status").and_then(Value::as_str),
                "details": details
            }),
        )
    }

    pub fn append_jsonl(&self, relative: &str, value: Value) -> Result<(), TaskRunError> {
        let path = self.task_root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        let mut line = serde_json::to_string(&value)
            .map_err(|error| TaskRunError::new("web_runtime_json", error.to_string()))?;
        line.push('\n');
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(io_error)?
            .write_all(line.as_bytes())
            .map_err(io_error)
    }
}

fn io_error(error: std::io::Error) -> TaskRunError {
    TaskRunError::new("web_runtime_io", error.to_string())
}
