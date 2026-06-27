use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id};

mod attempt;
mod context;
mod gate;
mod group;
mod inputs;
mod paths;
mod report;
mod role_run;
mod role_run_event;
mod timeline;
mod utils;

pub use inputs::*;
pub(crate) use utils::*;

#[derive(Debug, Clone)]
pub struct CodingAttemptStore {
    paths: ProductAppPaths,
}

impl CodingAttemptStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn paths(&self) -> ProductAppPaths {
        self.paths.clone()
    }

    pub(crate) fn find_attempt_by_id(
        &self,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        validate_relative_id(attempt_id)?;
        let mut found = None;
        for project_path in child_directories(&self.paths.projects_root())? {
            let Some(project_id) = project_path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let issues_root = project_path.join("issues");
            for issue_path in child_directories(&issues_root)? {
                let Some(issue_id) = issue_path.file_name().and_then(|value| value.to_str()) else {
                    continue;
                };
                let path = self.attempt_path(project_id, issue_id, attempt_id);
                if !path_is_regular_file(&path)? {
                    continue;
                }
                if found.is_some() {
                    return Err(ProductStoreError::Io(format!(
                        "coding_attempt_ambiguous: {attempt_id}"
                    )));
                }
                found = Some(read_json(&path)?);
            }
        }
        found.ok_or_else(|| ProductStoreError::NotFound {
            kind: "coding_attempt",
            id: attempt_id.to_string(),
        })
    }
}

#[cfg(test)]
mod tests;
