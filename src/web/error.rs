use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApiError {
    pub code: String,
    pub message: String,
    pub details: Value,
}

impl ApiError {
    pub fn validation(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: json!({}),
        }
    }

    pub fn runtime(code: impl Into<String>, message: impl Into<String>, details: Value) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.code.as_str() {
            "invalid_task_request" => StatusCode::BAD_REQUEST,
            "checkpoint_unsafe_dirty_worktree" => StatusCode::CONFLICT,
            "artifact_not_found"
            | "gate_not_found"
            | "interactive_task_missing"
            | "issue_not_found"
            | "project_not_found"
            | "workspace_not_found"
            | "task_workspace_not_found" => StatusCode::NOT_FOUND,
            "gate_ambiguous"
            | "invalid_file_path"
            | "invalid_project_id"
            | "invalid_task_id"
            | "issue_title_required"
            | "provider_input_path_escape"
            | "workspace_path_missing"
            | "workspace_path_not_directory"
            | "workspace_path_not_git_repo" => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(self)).into_response()
    }
}

impl From<crate::task_run::types::TaskRunError> for ApiError {
    fn from(error: crate::task_run::types::TaskRunError) -> Self {
        ApiError::runtime(error.code, error.message, json!({}))
    }
}

impl From<crate::web::workspace_registry::WorkspaceRegistryError> for ApiError {
    fn from(error: crate::web::workspace_registry::WorkspaceRegistryError) -> Self {
        ApiError::runtime(error.code(), error.message(), json!({}))
    }
}

impl From<crate::web::issue_registry::IssueRegistryError> for ApiError {
    fn from(error: crate::web::issue_registry::IssueRegistryError) -> Self {
        ApiError::runtime(error.code(), error.message(), json!({}))
    }
}
