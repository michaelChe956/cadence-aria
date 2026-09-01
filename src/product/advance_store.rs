use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::locking::with_exclusive_lock;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdvanceStatus {
    Initializing,
    Ready,
    Running,
    AwaitingPlanAmendment,
    Completed,
    Failed,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvanceRecord {
    pub id: String,
    pub command_id: String,
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub plan_revision_id: String,
    pub attempt_id: Option<String>,
    pub status: AdvanceStatus,
    pub workspace_entry: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvanceInput {
    pub command_id: String,
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdvanceOutcome {
    Completed {
        record: AdvanceRecord,
        attempt_id: String,
        workspace_entry: String,
    },
    Replayed {
        record: AdvanceRecord,
    },
    Rejected {
        record: Option<AdvanceRecord>,
        code: String,
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub struct AdvanceStore {
    app_paths: ProductAppPaths,
}

impl AdvanceStore {
    pub fn new(app_paths: ProductAppPaths) -> Self {
        Self { app_paths }
    }

    pub fn app_paths(&self) -> ProductAppPaths {
        self.app_paths.clone()
    }

    fn root(&self, project_id: &str, issue_id: &str) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        Ok(self
            .app_paths
            .issue_root(project_id, issue_id)
            .join("advance-records"))
    }

    fn path_for(
        &self,
        project_id: &str,
        issue_id: &str,
        id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(id)?;
        Ok(self.root(project_id, issue_id)?.join(format!("{id}.json")))
    }

    fn with_existing_root<T>(
        &self,
        project_id: &str,
        issue_id: &str,
        operation: impl FnOnce() -> Result<T, ProductStoreError>,
    ) -> Result<T, ProductStoreError> {
        let root = self.root(project_id, issue_id)?;
        if !root.is_dir() {
            return operation();
        }
        with_exclusive_lock(&root, operation)
    }

    fn records(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<AdvanceRecord>, ProductStoreError> {
        let root = self.root(project_id, issue_id)?;
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read {}: {error}",
                    root.display()
                )));
            }
        };
        let mut records = Vec::new();
        for entry in entries {
            let path = entry
                .map_err(|error| {
                    ProductStoreError::Io(format!("read advance record entry: {error}"))
                })?
                .path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let record: AdvanceRecord = read_json(&path)?;
            if record.project_id != project_id || record.issue_id != issue_id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "advance_record",
                    id: record.id,
                });
            }
            records.push(record);
        }
        records.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(records)
    }

    pub fn get_advance_by_command_id(
        &self,
        project_id: &str,
        issue_id: &str,
        command_id: &str,
    ) -> Result<Option<AdvanceRecord>, ProductStoreError> {
        validate_relative_id(command_id)?;
        self.with_existing_root(project_id, issue_id, || {
            Ok(self
                .records(project_id, issue_id)?
                .into_iter()
                .find(|record| record.command_id == command_id))
        })
    }

    pub fn get_advance_for_plan(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<Option<AdvanceRecord>, ProductStoreError> {
        validate_relative_id(plan_id)?;
        self.with_existing_root(project_id, issue_id, || {
            Ok(self
                .records(project_id, issue_id)?
                .into_iter()
                .find(|record| record.plan_id == plan_id))
        })
    }

    /// Test/next-stage orchestration hook.  Task 5.1 deliberately never calls this
    /// on a first request; Task 5.2 owns the first durable record write.
    pub fn put_record(&self, record: &AdvanceRecord) -> Result<AdvanceRecord, ProductStoreError> {
        validate_relative_id(&record.id)?;
        validate_relative_id(&record.command_id)?;
        validate_relative_id(&record.project_id)?;
        validate_relative_id(&record.issue_id)?;
        validate_relative_id(&record.plan_id)?;
        validate_relative_id(&record.plan_revision_id)?;
        let path = self.path_for(&record.project_id, &record.issue_id, &record.id)?;
        let root = self.root(&record.project_id, &record.issue_id)?;
        with_exclusive_lock(&root, || {
            if path.exists() {
                let existing: AdvanceRecord = read_json(&path)?;
                if existing == *record {
                    return Ok(existing);
                }
                return Err(ProductStoreError::Conflict {
                    kind: "advance_record",
                    id: record.id.clone(),
                });
            }
            if self
                .records(&record.project_id, &record.issue_id)?
                .into_iter()
                .any(|existing| {
                    existing.command_id == record.command_id || existing.plan_id == record.plan_id
                })
            {
                return Err(ProductStoreError::Conflict {
                    kind: "advance_record_identity",
                    id: record.id.clone(),
                });
            }
            write_json(&path, record)?;
            Ok(record.clone())
        })
    }

    pub fn now_record(&self, input: &AdvanceInput, plan_revision_id: String) -> AdvanceRecord {
        let now = Utc::now().to_rfc3339();
        AdvanceRecord {
            id: format!("advance_{}", input.command_id),
            command_id: input.command_id.clone(),
            project_id: input.project_id.clone(),
            issue_id: input.issue_id.clone(),
            plan_id: input.plan_id.clone(),
            plan_revision_id,
            attempt_id: None,
            status: AdvanceStatus::Initializing,
            workspace_entry: None,
            error: None,
            created_at: now.clone(),
            updated_at: now,
        }
    }
}

pub use AdvanceRecord as Record;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn record() -> AdvanceRecord {
        AdvanceRecord {
            id: "advance_0001".into(),
            command_id: "command_0001".into(),
            project_id: "project_0001".into(),
            issue_id: "issue_0001".into(),
            plan_id: "plan_0001".into(),
            plan_revision_id: "revision_0001".into(),
            attempt_id: Some("attempt_0001".into()),
            status: AdvanceStatus::Ready,
            workspace_entry: Some("/workspaces/attempt_0001".into()),
            error: None,
            created_at: "2026-08-31T00:00:00Z".into(),
            updated_at: "2026-08-31T00:00:01Z".into(),
        }
    }

    #[test]
    fn advance_record_store_supports_both_idempotency_indexes() {
        let root = TempDir::new().unwrap();
        let store = AdvanceStore::new(ProductAppPaths::new(root.path().join(".aria")));
        let value = record();
        assert_eq!(store.put_record(&value).unwrap(), value);
        assert_eq!(
            store
                .get_advance_by_command_id("project_0001", "issue_0001", "command_0001")
                .unwrap(),
            Some(value.clone())
        );
        assert_eq!(
            store
                .get_advance_for_plan("project_0001", "issue_0001", "plan_0001")
                .unwrap(),
            Some(value)
        );
    }
}
