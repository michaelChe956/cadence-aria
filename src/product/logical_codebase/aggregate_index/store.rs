use chrono::Utc;

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

use super::{AggregateIndexError, AggregateIndexRecord, AggregateIndexStatus};

/// Durable aggregate-index records, isolated beneath their owning project.
#[derive(Debug, Clone)]
pub struct AggregateIndexStore {
    paths: ProductAppPaths,
}

impl AggregateIndexStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn create(
        &self,
        project_id: &str,
        record: AggregateIndexRecord,
    ) -> Result<AggregateIndexRecord, AggregateIndexError> {
        self.validate_record(project_id, &record)?;
        let path = self.record_path(project_id, &record.aggregate_index_id)?;
        if path
            .try_exists()
            .map_err(|error| store_io_error(&path, error))?
        {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_already_exists",
                message: format!(
                    "aggregate index {} already exists for project {project_id}",
                    record.aggregate_index_id
                ),
            });
        }
        write_json(&path, &record)?;
        Ok(record)
    }

    pub fn get(
        &self,
        project_id: &str,
        aggregate_index_id: &str,
    ) -> Result<Option<AggregateIndexRecord>, AggregateIndexError> {
        let path = self.record_path(project_id, aggregate_index_id)?;
        if !path
            .try_exists()
            .map_err(|error| store_io_error(&path, error))?
        {
            return Ok(None);
        }
        let record: AggregateIndexRecord = read_json(&path)?;
        self.validate_record(project_id, &record)?;
        if record.aggregate_index_id != aggregate_index_id {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_identity_mismatch",
                message: format!(
                    "aggregate-index record at {} is {} rather than {aggregate_index_id}",
                    path.display(),
                    record.aggregate_index_id
                ),
            });
        }
        Ok(Some(record))
    }

    pub fn active(
        &self,
        project_id: &str,
    ) -> Result<Option<AggregateIndexRecord>, AggregateIndexError> {
        validate_relative_id(project_id)?;
        let records = self.records(project_id)?;
        let active = records
            .into_iter()
            .filter(|record| record.status == AggregateIndexStatus::Active)
            .collect::<Vec<_>>();
        match active.as_slice() {
            [] => Ok(None),
            [record] => Ok(Some(record.clone())),
            _ => Err(AggregateIndexError::Failed {
                code: "aggregate_index_active_ambiguous",
                message: format!("project {project_id} has multiple active aggregate indexes"),
            }),
        }
    }

    /// Replaces the active record only after the caller has completed build and
    /// acceptance. Task 7 will refine replacement ordering for last-known-good
    /// recovery; this initial store deliberately persists the transition.
    pub fn replace_active(
        &self,
        project_id: &str,
        mut next: AggregateIndexRecord,
    ) -> Result<AggregateIndexRecord, AggregateIndexError> {
        validate_relative_id(project_id)?;
        self.validate_record(project_id, &next)?;
        if next.status != AggregateIndexStatus::Active {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_invalid_status_transition",
                message: "replace_active requires an active aggregate-index record".to_string(),
            });
        }

        let current = self.active(project_id)?;
        if let Some(mut previous) = current
            && previous.aggregate_index_id != next.aggregate_index_id
        {
            previous.status = AggregateIndexStatus::Superseded;
            previous.updated_at = Utc::now().to_rfc3339();
            self.save(project_id, &previous)?;
            next.supersedes_aggregate_index_id = Some(previous.aggregate_index_id);
        }
        self.save(project_id, &next)?;
        Ok(next)
    }

    pub fn mark_status(
        &self,
        project_id: &str,
        aggregate_index_id: &str,
        status: AggregateIndexStatus,
        warning: Option<String>,
    ) -> Result<AggregateIndexRecord, AggregateIndexError> {
        let mut record = self.get(project_id, aggregate_index_id)?.ok_or_else(|| {
            AggregateIndexError::Failed {
                code: "aggregate_index_not_found",
                message: format!("aggregate index {aggregate_index_id} was not found"),
            }
        })?;
        record.status = status;
        record.warning = warning;
        record.updated_at = Utc::now().to_rfc3339();
        self.save(project_id, &record)?;
        Ok(record)
    }

    fn records(&self, project_id: &str) -> Result<Vec<AggregateIndexRecord>, AggregateIndexError> {
        validate_relative_id(project_id)?;
        let root = self.paths.aggregate_indexes_root(project_id);
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(store_io_error(&root, error)),
        };

        let mut paths = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| store_io_error(&root, error))?;
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            if !entry
                .file_type()
                .map_err(|error| store_io_error(&path, error))?
                .is_file()
            {
                return Err(AggregateIndexError::Failed {
                    code: "aggregate_index_invalid_record_path",
                    message: format!(
                        "aggregate-index record path is not a regular file: {}",
                        path.display()
                    ),
                });
            }
            let aggregate_index_id =
                path.file_stem()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| AggregateIndexError::Failed {
                        code: "aggregate_index_invalid_record_path",
                        message: format!(
                            "aggregate-index record name is not UTF-8: {}",
                            path.display()
                        ),
                    })?;
            validate_relative_id(aggregate_index_id)?;
            paths.push((aggregate_index_id.to_string(), path));
        }
        paths.sort_by(|left, right| left.0.cmp(&right.0));

        let mut records = Vec::with_capacity(paths.len());
        for (aggregate_index_id, path) in paths {
            let record: AggregateIndexRecord = read_json(&path)?;
            self.validate_record(project_id, &record)?;
            if record.aggregate_index_id != aggregate_index_id {
                return Err(AggregateIndexError::Failed {
                    code: "aggregate_index_identity_mismatch",
                    message: format!(
                        "aggregate-index record at {} is {} rather than {aggregate_index_id}",
                        path.display(),
                        record.aggregate_index_id
                    ),
                });
            }
            records.push(record);
        }
        Ok(records)
    }

    fn save(
        &self,
        project_id: &str,
        record: &AggregateIndexRecord,
    ) -> Result<(), AggregateIndexError> {
        self.validate_record(project_id, record)?;
        write_json(
            &self.record_path(project_id, &record.aggregate_index_id)?,
            record,
        )?;
        Ok(())
    }

    fn record_path(
        &self,
        project_id: &str,
        aggregate_index_id: &str,
    ) -> Result<std::path::PathBuf, AggregateIndexError> {
        validate_relative_id(project_id)?;
        validate_relative_id(aggregate_index_id)?;
        Ok(self
            .paths
            .aggregate_indexes_root(project_id)
            .join(format!("{aggregate_index_id}.json")))
    }

    fn validate_record(
        &self,
        project_id: &str,
        record: &AggregateIndexRecord,
    ) -> Result<(), AggregateIndexError> {
        validate_relative_id(project_id)?;
        validate_relative_id(&record.project_id)?;
        validate_relative_id(&record.aggregate_index_id)?;
        if record.project_id != project_id {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_project_mismatch",
                message: format!(
                    "aggregate index {} belongs to project {} rather than {project_id}",
                    record.aggregate_index_id, record.project_id
                ),
            });
        }
        Ok(())
    }
}

fn store_io_error(path: &std::path::Path, error: std::io::Error) -> AggregateIndexError {
    ProductStoreError::Io(format!("read {}: {error}", path.display())).into()
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use uuid::Uuid;

    use super::*;

    #[test]
    fn replace_active_supersedes_the_prior_record_and_mark_status_persists() {
        let temp = tempfile::tempdir().unwrap();
        let store = AggregateIndexStore::new(ProductAppPaths::new(temp.path()));
        let first = record("aggregate_index_first");
        let second = record("aggregate_index_second");

        store.replace_active("project_0001", first.clone()).unwrap();
        let active = store
            .replace_active("project_0001", second.clone())
            .unwrap();

        assert_eq!(
            active.supersedes_aggregate_index_id.as_deref(),
            Some("aggregate_index_first")
        );
        assert_eq!(
            store
                .get("project_0001", "aggregate_index_first")
                .unwrap()
                .unwrap()
                .status,
            AggregateIndexStatus::Superseded
        );
        let marked = store
            .mark_status(
                "project_0001",
                "aggregate_index_second",
                AggregateIndexStatus::Degraded,
                Some("CodeGraph unavailable".to_string()),
            )
            .unwrap();
        assert_eq!(marked.status, AggregateIndexStatus::Degraded);
        assert_eq!(marked.warning.as_deref(), Some("CodeGraph unavailable"));
    }

    #[test]
    fn active_rejects_record_file_name_that_does_not_match_its_id() {
        let temp = tempfile::tempdir().unwrap();
        let store = AggregateIndexStore::new(ProductAppPaths::new(temp.path()));
        let record = record("aggregate_index_actual");
        let path = temp.path().join(
            "projects/project_0001/logical-codebase/aggregate-indexes/aggregate_index_other.json",
        );
        write_json(&path, &record).unwrap();

        assert!(matches!(
            store.active("project_0001"),
            Err(AggregateIndexError::Failed { code, .. }) if code == "aggregate_index_identity_mismatch"
        ));
    }

    #[test]
    fn store_rejects_project_and_index_path_escapes() {
        let temp = tempfile::tempdir().unwrap();
        let store = AggregateIndexStore::new(ProductAppPaths::new(temp.path()));

        assert!(matches!(
            store.get("../project", "aggregate_index_0001"),
            Err(AggregateIndexError::Failed { code, .. }) if code == "aggregate_index_store_error"
        ));
        assert!(matches!(
            store.get("project_0001", "../aggregate_index_0001"),
            Err(AggregateIndexError::Failed { code, .. }) if code == "aggregate_index_store_error"
        ));
    }

    fn record(id: &str) -> AggregateIndexRecord {
        let now = Utc::now().to_rfc3339();
        let mut record = AggregateIndexRecord::building(
            id.to_string(),
            "project_0001".to_string(),
            3,
            vec![super::super::AggregateIndexMemberSnapshot::indexed(
                crate::product::logical_codebase::LogicalRepositoryId(Uuid::new_v4()),
                crate::product::logical_codebase::RepositoryCheckoutId(Uuid::new_v4()),
                "a".repeat(40),
                false,
                now.clone(),
            )],
            now,
        );
        record.status = AggregateIndexStatus::Active;
        record
    }
}
