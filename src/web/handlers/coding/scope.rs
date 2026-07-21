use serde::Deserialize;

use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::CodingExecutionAttempt;
use crate::web::error::{ApiError, ApiResult};

use super::super::support::product_store_api_error;

#[derive(Debug, Deserialize)]
pub(crate) struct CodingAttemptRoutePath {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub issue_id: Option<String>,
    pub attempt_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CodingAttemptArtifactRoutePath {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub issue_id: Option<String>,
    pub attempt_id: String,
    pub artifact_id: String,
}

pub(crate) fn resolve_coding_attempt(
    store: &CodingAttemptStore,
    project_id: Option<&str>,
    issue_id: Option<&str>,
    attempt_id: &str,
) -> ApiResult<CodingExecutionAttempt> {
    match (project_id, issue_id) {
        (Some(project_id), Some(issue_id)) => store
            .get_attempt(project_id, issue_id, attempt_id)
            .map_err(product_store_api_error),
        (None, None) => store
            .get_attempt_by_id(attempt_id)
            .map_err(product_store_api_error),
        _ => Err(ApiError::validation(
            "invalid_coding_attempt_scope",
            "project_id and issue_id must be provided together",
        )),
    }
}
