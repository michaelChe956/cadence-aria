use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::product::app_paths::ProductAppPaths;
use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::models::{ExecutionRecord, ExecutionStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendExecutionRecordInput {
    pub project_id: String,
    pub issue_id: String,
    pub binding_id: String,
    pub node_id: String,
    pub status: ExecutionStatus,
    pub event_type: String,
    pub artifact_refs: Vec<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ExecutionRecordStore {
    paths: ProductAppPaths,
}

impl ExecutionRecordStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn append(
        &self,
        input: AppendExecutionRecordInput,
    ) -> Result<ExecutionRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.binding_id)?;
        validate_relative_id(&input.node_id)?;

        let path = self.execution_records_path(&input.project_id, &input.issue_id);
        let existing_len = count_jsonl_records(&path)?;
        let now = Utc::now().to_rfc3339();
        let record = ExecutionRecord {
            id: next_sequential_id("execution", existing_len),
            project_id: input.project_id,
            issue_id: input.issue_id,
            binding_id: input.binding_id,
            node_id: input.node_id,
            status: input.status,
            event_type: input.event_type,
            artifact_refs: input.artifact_refs,
            message: input.message,
            created_at: now,
        };

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                ProductStoreError::Io(format!("create {}: {error}", parent.display()))
            })?;
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| ProductStoreError::Io(format!("open {}: {error}", path.display())))?;
        serde_json::to_writer(&mut file, &record)
            .map_err(|error| ProductStoreError::Json(error.to_string()))?;
        file.write_all(b"\n")
            .map_err(|error| ProductStoreError::Io(format!("write {}: {error}", path.display())))?;
        file.flush()
            .map_err(|error| ProductStoreError::Io(format!("flush {}: {error}", path.display())))?;
        Ok(record)
    }

    fn execution_records_path(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_root(project_id, issue_id)
            .join("execution-records.jsonl")
    }
}

fn count_jsonl_records(path: &Path) -> Result<usize, ProductStoreError> {
    if !path.exists() {
        return Ok(0);
    }
    let content = fs::read_to_string(path)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))?;
    Ok(content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count())
}
