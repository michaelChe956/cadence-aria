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
            "repository_project_not_found" => StatusCode::NOT_FOUND,
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
            | "work_item_group_empty"
            | "repository_path_not_git_repo"
            | "repository_path_invalid"
            | "repository_not_git"
            | "work_item_split_invalid" => StatusCode::BAD_REQUEST,
            "issue_worktree_active"
            | "repository_already_registered"
            | "repository_initialization_in_progress"
            | "shared_worktree_dirty_manual_gate" => StatusCode::CONFLICT,
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

impl From<crate::product::repository_store::RepositoryRegistrationError> for ApiError {
    fn from(error: crate::product::repository_store::RepositoryRegistrationError) -> Self {
        let stderr_summary = error
            .stderr_summary
            .as_deref()
            .map(sanitize_repository_api_text);
        ApiError::runtime(
            error.reason_code.clone(),
            "repository registration failed",
            json!({
                "stage": error.stage,
                "provider": error.provider,
                "command": error.command,
                "reason_code": error.reason_code,
                "stderr_summary": stderr_summary,
                "changed_paths": error.changed_paths.unwrap_or_default(),
                "retryable": error.retryable,
                "action": error.action,
            }),
        )
    }
}

pub(crate) fn sanitize_repository_api_text(value: impl AsRef<str>) -> String {
    const LIMIT: usize = 1_024;

    let normalized = value
        .as_ref()
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let mut sanitized = Vec::new();
    let mut redact_next = false;

    for token in normalized.split_whitespace() {
        if redact_next {
            sanitized.push("<redacted>".to_string());
            redact_next = false;
            continue;
        }

        if let Some((key, value)) = token.split_once('=') {
            if is_sensitive_key(key) {
                sanitized.push(format!("{key}=<redacted>"));
            } else if is_absolute_path_token(value) {
                sanitized.push(format!("{key}=<path>"));
            } else {
                sanitized.push(token.to_string());
            }
            continue;
        }

        let key = token.trim_end_matches(':');
        if is_sensitive_key(key) {
            sanitized.push(format!("{key}:<redacted>"));
            redact_next = token.ends_with(':');
        } else if is_absolute_path_token(token) {
            sanitized.push("<path>".to_string());
        } else {
            sanitized.push(token.to_string());
        }
    }

    redact_absolute_paths(&sanitized.join(" "))
        .chars()
        .take(LIMIT)
        .collect::<String>()
}

pub(crate) fn sanitize_repository_api_warnings(warnings: Vec<String>) -> Vec<String> {
    warnings
        .into_iter()
        .map(sanitize_repository_api_text)
        .collect()
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_uppercase();
    ["KEY", "TOKEN", "SECRET", "PASSWORD"]
        .iter()
        .any(|needle| key.contains(needle))
}

fn is_absolute_path_token(token: &str) -> bool {
    let token = token.trim_matches(|character: char| {
        matches!(character, '"' | '\'' | '(' | ')' | '[' | ']' | ',' | ';')
    });
    token.starts_with('/')
        || token.starts_with('\\')
        || (token.len() >= 3
            && token.as_bytes()[0].is_ascii_alphabetic()
            && token.as_bytes()[1] == b':'
            && matches!(token.as_bytes()[2], b'/' | b'\\'))
}

fn redact_absolute_paths(value: &str) -> String {
    let mut redacted = String::with_capacity(value.len());
    let mut cursor = 0;
    while cursor < value.len() {
        if value[cursor..].starts_with("file://") {
            redacted.push_str("file://<path>");
            cursor += "file://".len();
            cursor = consume_path(value, cursor);
            continue;
        }
        if is_absolute_path_start(value, cursor) {
            redacted.push_str("<path>");
            cursor = consume_path(value, cursor);
            continue;
        }
        let character = value[cursor..].chars().next().expect("character");
        redacted.push(character);
        cursor += character.len_utf8();
    }
    redacted
}

fn is_absolute_path_start(value: &str, cursor: usize) -> bool {
    let remaining = &value[cursor..];
    let boundary = cursor == 0
        || value[..cursor]
            .chars()
            .next_back()
            .is_some_and(is_path_boundary);
    if !boundary {
        return false;
    }
    if remaining.starts_with('/') {
        return !value[..cursor].ends_with("http:") && !value[..cursor].ends_with("https:");
    }
    if remaining.starts_with('\\') {
        return true;
    }
    let bytes = remaining.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
}

fn consume_path(value: &str, mut cursor: usize) -> usize {
    while cursor < value.len() {
        let character = value[cursor..].chars().next().expect("path character");
        if is_path_terminator(character) {
            break;
        }
        cursor += character.len_utf8();
    }
    cursor
}

fn is_path_boundary(character: char) -> bool {
    character.is_whitespace()
        || matches!(
            character,
            '"' | '\'' | '(' | '[' | '{' | '=' | ':' | ',' | ';'
        )
}

fn is_path_terminator(character: char) -> bool {
    character.is_whitespace() || matches!(character, '"' | '\'' | ')' | ']' | '}' | ',' | ';')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::repository_store::RepositoryRegistrationError;

    #[test]
    fn work_item_group_empty_is_bad_request() {
        let response = ApiError::validation(
            "work_item_group_empty",
            "work item group has no compiled work items",
        )
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn repository_registration_api_error_maps_all_codes_and_stable_details() {
        let cases = [
            ("repository_project_not_found", StatusCode::NOT_FOUND),
            ("repository_path_invalid", StatusCode::BAD_REQUEST),
            ("repository_not_git", StatusCode::BAD_REQUEST),
            ("repository_already_registered", StatusCode::CONFLICT),
            (
                "repository_initialization_in_progress",
                StatusCode::CONFLICT,
            ),
            ("provider_unavailable", StatusCode::INTERNAL_SERVER_ERROR),
            (
                "host_real_workflow_blocked",
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (
                "cadence_skills_unavailable",
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (
                "cadence_skills_update_failed",
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (
                "cadence_skills_sync_failed",
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (
                "repository_git_state_failed",
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (
                "repository_init_command_failed",
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (
                "repository_init_interaction_required",
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (
                "repository_persist_failed",
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ];

        for (code, expected_status) in cases {
            let api_error = ApiError::from(RepositoryRegistrationError {
                stage: "repository_init_command".to_string(),
                provider: Some("claude_code".to_string()),
                command_index: Some(2),
                command: Some("/rule-config".to_string()),
                reason_code: code.to_string(),
                stderr_summary: Some("safe diagnostic".to_string()),
                changed_paths: Some(vec![".claude/rules/project.md".to_string()]),
                retryable: true,
                action: "Fix the problem, inspect changed_paths, then add the repository again."
                    .to_string(),
            });

            assert_eq!(api_error.code, code);
            assert_eq!(
                api_error.details,
                json!({
                    "stage": "repository_init_command",
                    "provider": "claude_code",
                    "command": "/rule-config",
                    "reason_code": code,
                    "stderr_summary": "safe diagnostic",
                    "changed_paths": [".claude/rules/project.md"],
                    "retryable": true,
                    "action": "Fix the problem, inspect changed_paths, then add the repository again."
                })
            );
            assert_eq!(
                api_error.into_response().status(),
                expected_status,
                "{code}"
            );
        }
    }

    #[test]
    fn repository_registration_api_error_uses_nulls_and_empty_changed_paths() {
        let api_error = ApiError::from(RepositoryRegistrationError {
            stage: "repository_path".to_string(),
            provider: None,
            command_index: None,
            command: None,
            reason_code: "repository_path_invalid".to_string(),
            stderr_summary: None,
            changed_paths: None,
            retryable: false,
            action: "Choose a valid path, then add the repository again.".to_string(),
        });

        assert_eq!(api_error.details["provider"], Value::Null);
        assert_eq!(api_error.details["command"], Value::Null);
        assert_eq!(api_error.details["stderr_summary"], Value::Null);
        assert_eq!(api_error.details["changed_paths"], json!([]));
    }

    #[test]
    fn repository_registration_api_error_sanitizes_public_summary() {
        let api_error = ApiError::from(RepositoryRegistrationError {
            stage: "repository_init_command".to_string(),
            provider: Some("claude_code".to_string()),
            command_index: Some(1),
            command: Some("/pre-check".to_string()),
            reason_code: "repository_init_command_failed".to_string(),
            stderr_summary: Some(format!(
                "failed\nTOKEN=super-secret PASSWORD: hidden HOME=/home/alice /usr/bin/git json={{\"path\":\"/home/alice/repo\",\"command\":\"/usr/bin/git\"}} file:///home/alice/repo failed_at(/home/alice/file.rs:10) {}",
                "x".repeat(2_000)
            )),
            changed_paths: Some(Vec::new()),
            retryable: true,
            action: "Fix the problem, then add the repository again.".to_string(),
        });
        let summary = api_error.details["stderr_summary"]
            .as_str()
            .expect("sanitized stderr summary");

        assert!(summary.starts_with("failed"));
        assert!(!summary.contains("super-secret"));
        assert!(!summary.contains("hidden"));
        assert!(!summary.contains("/home/alice"));
        assert!(!summary.contains("/usr/bin/git"));
        assert!(!summary.contains("file:///"));
        assert!(!summary.contains("file.rs:10"));
        assert!(summary.chars().count() <= 1_024);
        assert!(!summary.chars().any(char::is_control));
    }
}
