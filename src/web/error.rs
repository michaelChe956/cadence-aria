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

    pub fn validation_with_details(
        code: impl Into<String>,
        message: impl Into<String>,
        details: Value,
    ) -> Self {
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
            "checkpoint_unsafe_dirty_worktree" | "workspace_session_ambiguous" => {
                StatusCode::CONFLICT
            }
            "coding_attempt_active" | "coding_attempt_worktree_not_ready" => StatusCode::CONFLICT,
            "artifact_not_found"
            | "artifact_version_not_found"
            | "coding_attempt_not_found"
            | "event_output_not_found"
            | "gate_not_found"
            | "interactive_task_missing"
            | "issue_not_found"
            | "node_detail_not_found"
            | "node_detail_prompt_not_found"
            | "project_not_found"
            | "repository_not_found"
            | "workspace_not_found"
            | "work_item_not_found"
            | "task_workspace_not_found"
            | "workspace_session_not_found" => StatusCode::NOT_FOUND,
            "gate_ambiguous"
            | "invalid_execution_record_id"
            | "invalid_artifact_id"
            | "invalid_file_path"
            | "invalid_issue_id"
            | "invalid_project_id"
            | "invalid_workspace_message"
            | "invalid_task_id"
            | "issue_rollback_missing_worktree"
            | "issue_title_required"
            | "project_required"
            | "provider_input_path_escape"
            | "repository_required"
            | "workspace_path_missing"
            | "workspace_path_not_directory"
            | "workspace_path_not_git_repo"
            | "work_item_plan_not_confirmed"
            | "work_item_dependency_not_completed"
            | "work_item_handoff_missing"
            | "work_item_execution_plan_not_confirmed"
            | "repository_path_not_git_repo"
            | "work_item_split_invalid" => StatusCode::BAD_REQUEST,
            "issue_worktree_active" | "shared_worktree_dirty_manual_gate" => StatusCode::CONFLICT,
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
