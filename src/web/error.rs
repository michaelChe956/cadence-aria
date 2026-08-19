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
            "coding_attempt_active"
            | "coding_attempt_ambiguous"
            | "coding_attempt_scope_mismatch"
            | "coding_attempt_worktree_not_ready" => StatusCode::CONFLICT,
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
            | "repository_initialization_operation_not_found"
            | "spec_not_found"
            | "workspace_not_found"
            | "work_item_not_found"
            | "task_workspace_not_found"
            | "workspace_session_not_found"
            | "registration_preflight_not_found"
            | "registration_batch_not_found" => StatusCode::NOT_FOUND,
            "repository_project_not_found" | "repository_routing_target_unknown" => {
                StatusCode::NOT_FOUND
            }
            "repository_routing_target_missing"
            | "involved_repository_not_effective"
            | "change_order_repository_not_involved"
            | "change_order_duplicate_repository"
            | "mixed_target_group_rejected"
            | "target_snapshot_missing_for_logical"
            | "aggregate_index_unavailable"
            | "issue_selection_write_failed" => StatusCode::UNPROCESSABLE_ENTITY,
            "registration_batch_conflict"
            | "aggregate_initialization_conflict"
            | "aggregate_index_rebuild_in_progress"
            | "aggregate_root_mismatch"
            | "target_snapshot_identity_drifted"
            | "target_snapshot_policy_drifted"
            | "legacy_shared_worktree_present"
            | "legacy_shared_worktree_inconsistent"
            | "repo_worktree_active"
            | "cross_target_violation_detected"
            | "registration_batch_not_cancelable" => StatusCode::CONFLICT,
            "cross_target_baseline_missing" | "cross_target_store_failure" => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            "repository_routing_inconsistent" | "repository_routing_ambiguous" => {
                StatusCode::CONFLICT
            }
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
            | "schema_v2_group_coding_required"
            | "coding_plan_revision_binding_missing"
            | "coding_group_attempt_incomplete"
            | "work_item_dependency_not_completed"
            | "work_item_execution_plan_not_confirmed"
            | "work_item_group_empty"
            | "repository_path_not_git_repo"
            | "repository_path_invalid"
            | "repository_not_git"
            | "work_item_split_invalid"
            | "story_spec_not_confirmed"
            | "design_spec_not_confirmed"
            | "involved_repositories_undetermined"
            | "change_order_required_for_logical_codebase"
            | "aggregate_scope_requires_logical_codebase"
            | "target_not_in_selection" => StatusCode::BAD_REQUEST,
            "aggregate_root_is_git"
            | "aggregate_root_missing"
            | "aggregate_root_member_outside_root"
            | "aggregate_root_member_symlink_escape"
            | "aggregate_root_nested_worktree" => StatusCode::UNPROCESSABLE_ENTITY,
            "aggregate_root_internal_error" => StatusCode::INTERNAL_SERVER_ERROR,
            "issue_worktree_active"
            | "repository_already_registered"
            | "repository_initialization_in_progress"
            | "shared_worktree_dirty_manual_gate"
            | "coding_workspace_exists"
            | "legacy_repository_endpoint_on_multi_repo"
            | "logical_codebase_feature_disabled"
            | "aggregate_root_ownership_conflict" => StatusCode::CONFLICT,
            // T11 gateway 稳定码表收口：政策门/复验拒绝为 4xx 业务阻断，禁止回退 500。
            // policy/target/registry/drift/resume/managed_settings 一律 409；capability 与
            // Codex danger-full-access 为 403；provider 不可用为 503；系统未接线为 500(防御)。
            "provider_gateway_policy_missing"
            | "provider_gateway_policy"
            | "provider_gateway_target"
            | "provider_gateway_target_mismatch"
            | "provider_gateway_missing_cwd"
            | "provider_gateway_registry_lookup"
            | "provider_gateway_policy_drift"
            | "provider_gateway_resume_not_supported"
            | "provider_gateway_managed_settings_active" => StatusCode::CONFLICT,
            // `codex_danger_full_access_unsupported` 为防御性死映射:生产活跃路径已把
            // UnsupportedCapability("codex_danger_full_access_unsupported") 归一为
            // `provider_gateway_capability`(两者同映射 403)。保留此码以防有路径直接上抛该
            // reason 字符串。
            "provider_gateway_capability" | "codex_danger_full_access_unsupported" => {
                StatusCode::FORBIDDEN
            }
            "provider_gateway_unavailable" => StatusCode::SERVICE_UNAVAILABLE,
            "logical_provider_gateway_required" => StatusCode::INTERNAL_SERVER_ERROR,
            // Task 10 pointer 发布稳定码：busy/conflict 409、push/revoke 失败 503、
            // not_found 404、参数非法 422。
            "pointer_publish_busy" | "pointer_conflict_unresolved" => StatusCode::CONFLICT,
            "pointer_push_failed" | "pointer_revoke_failed" => StatusCode::SERVICE_UNAVAILABLE,
            "pointer_not_found" => StatusCode::NOT_FOUND,
            "invalid_pointer_request" => StatusCode::UNPROCESSABLE_ENTITY,
            // Task 7 证据查询稳定码：6 码 + evidence_io（设计 §5.2）。
            "evidence_unauthorized" => StatusCode::UNAUTHORIZED,
            "evidence_forbidden" => StatusCode::FORBIDDEN,
            "evidence_not_available" => StatusCode::NOT_FOUND,
            "evidence_invalid_query" => StatusCode::UNPROCESSABLE_ENTITY,
            "evidence_budget_exhausted" => StatusCode::TOO_MANY_REQUESTS,
            "evidence_query_failed" => StatusCode::SERVICE_UNAVAILABLE,
            "evidence_io" => StatusCode::INTERNAL_SERVER_ERROR,
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
                "changed_paths": sanitize_repository_api_changed_paths(
                    error.changed_paths.unwrap_or_default(),
                ),
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

pub(crate) fn sanitize_repository_api_path(value: impl AsRef<str>) -> String {
    let value = value.as_ref();
    if is_absolute_repository_api_path(value) {
        "<path>".to_string()
    } else {
        value.to_string()
    }
}

pub(crate) fn sanitize_repository_api_changed_paths(changed_paths: Vec<String>) -> Vec<String> {
    changed_paths
        .into_iter()
        .map(sanitize_repository_api_path)
        .collect()
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

fn is_absolute_repository_api_path(value: &str) -> bool {
    value.starts_with('/')
        || value.starts_with('\\')
        || (value.len() >= 3
            && value.as_bytes()[0].is_ascii_alphabetic()
            && value.as_bytes()[1] == b':'
            && matches!(value.as_bytes()[2], b'/' | b'\\'))
}

fn is_absolute_path_token(token: &str) -> bool {
    let token = token.trim_matches(|character: char| {
        matches!(character, '"' | '\'' | '(' | ')' | '[' | ']' | ',' | ';')
    });
    is_absolute_repository_api_path(token)
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
    fn repository_routing_error_codes_map_to_4xx() {
        // B3：稳定错误码必须 4xx 业务阻断，不是 500
        let cases = [
            (
                "repository_routing_target_missing",
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            ("repository_routing_inconsistent", StatusCode::CONFLICT),
            ("repository_routing_target_unknown", StatusCode::NOT_FOUND),
            ("repository_routing_ambiguous", StatusCode::CONFLICT),
        ];

        for (code, expected_status) in cases {
            let response = ApiError::validation(code, "routing fail-closed").into_response();
            assert_eq!(response.status(), expected_status, "{code} status mapping");
            assert!(response.status().is_client_error(), "{code} must be 4xx");
        }
    }
    #[test]
    fn gateway_error_mapping_covers_stable_codes() {
        // T11 §3 稳定码表收口：gateway 错误前缀 → HTTP 状态集中映射。
        // 表驱动断言每个稳定码的 HTTP 状态，防止未知码回退到 500。
        let cases = [
            ("provider_gateway_policy_missing", StatusCode::CONFLICT),
            ("provider_gateway_policy", StatusCode::CONFLICT),
            ("provider_gateway_target", StatusCode::CONFLICT),
            ("provider_gateway_target_mismatch", StatusCode::CONFLICT),
            ("provider_gateway_missing_cwd", StatusCode::CONFLICT),
            ("provider_gateway_capability", StatusCode::FORBIDDEN),
            (
                "codex_danger_full_access_unsupported",
                StatusCode::FORBIDDEN,
            ),
            ("provider_gateway_registry_lookup", StatusCode::CONFLICT),
            ("provider_gateway_policy_drift", StatusCode::CONFLICT),
            (
                "provider_gateway_resume_not_supported",
                StatusCode::CONFLICT,
            ),
            (
                "provider_gateway_managed_settings_active",
                StatusCode::CONFLICT,
            ),
            (
                "provider_gateway_unavailable",
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                "logical_provider_gateway_required",
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ];

        for (code, expected_status) in cases {
            let response = ApiError::validation(code, "gateway fail-closed").into_response();
            assert_eq!(response.status(), expected_status, "{code} status mapping");
        }
    }
    #[test]
    fn stable_code_http_contract_covers_all_legacy_and_routing_codes() {
        // §4.6 稳定码契约收口：未知码默认 500，必须显式声明。覆盖迁移与仓维路径引入的
        // 全部稳定码 HTTP 状态映射。
        let cases = [
            (
                "target_snapshot_missing_for_logical",
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            ("target_snapshot_identity_drifted", StatusCode::CONFLICT),
            ("target_snapshot_policy_drifted", StatusCode::CONFLICT),
            (
                "mixed_target_group_rejected",
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            ("legacy_shared_worktree_present", StatusCode::CONFLICT),
            ("legacy_shared_worktree_inconsistent", StatusCode::CONFLICT),
            ("repo_worktree_active", StatusCode::CONFLICT),
            ("cross_target_violation_detected", StatusCode::CONFLICT),
            (
                "cross_target_baseline_missing",
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (
                "cross_target_store_failure",
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (
                "legacy_repository_endpoint_on_multi_repo",
                StatusCode::CONFLICT,
            ),
            ("logical_codebase_feature_disabled", StatusCode::CONFLICT),
        ];

        for (code, expected_status) in cases {
            let response = ApiError::validation(code, "stable code contract").into_response();
            assert_eq!(response.status(), expected_status, "{code} status mapping");
        }
    }
    #[test]
    fn mixed_target_group_rejected_maps_to_422() {
        // REQ-COD-04（Task 12）：mixed-target group 拒绝码必须 422 UNPROCESSABLE_ENTITY，
        // 不是 500（未知码默认 500，必须显式声明）。
        let response = ApiError::validation("mixed_target_group_rejected", "mixed-target group")
            .into_response();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(response.status().is_client_error(), "must be 4xx");
    }
    #[test]
    fn confirm_gate_error_codes_map_to_4xx() {
        // Task 6 confirm gate：多仓 involved 缺失 / 多仓 Design 缺 change_order 必须是 4xx
        // 业务阻断（不是 500），与 brief 的 "4xx blocker" 一致。
        let cases = [
            (
                "involved_repositories_undetermined",
                StatusCode::BAD_REQUEST,
            ),
            (
                "change_order_required_for_logical_codebase",
                StatusCode::BAD_REQUEST,
            ),
        ];

        for (code, expected_status) in cases {
            let response = ApiError::validation(code, "confirm gate fail-closed").into_response();
            assert_eq!(response.status(), expected_status, "{code} status mapping");
            assert!(response.status().is_client_error(), "{code} must be 4xx");
        }
    }
    #[test]
    fn work_item_plan_target_not_in_selection_is_bad_request() {
        // Task 7 prepare_work_item_plan Logical 分支：design involved ∉ selection 必须是 4xx
        // 业务阻断（REQ-TGT-01，不是 500）。
        let response = ApiError::validation("target_not_in_selection", "target not in selection")
            .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(response.status().is_client_error(), "must be 4xx");
    }
    #[test]
    fn work_item_group_empty_is_bad_request() {
        let response = ApiError::validation(
            "work_item_group_empty",
            "work item group has no compiled work items",
        )
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn evidence_stable_codes_map_to_design_http_status() {
        // 设计 §5.2：evidence_* 稳定码 → HTTP 状态集中登记。
        let cases = [
            ("evidence_unauthorized", StatusCode::UNAUTHORIZED),
            ("evidence_forbidden", StatusCode::FORBIDDEN),
            ("evidence_not_available", StatusCode::NOT_FOUND),
            ("evidence_invalid_query", StatusCode::UNPROCESSABLE_ENTITY),
            ("evidence_budget_exhausted", StatusCode::TOO_MANY_REQUESTS),
            ("evidence_query_failed", StatusCode::SERVICE_UNAVAILABLE),
            ("evidence_io", StatusCode::INTERNAL_SERVER_ERROR),
        ];

        for (code, expected) in cases {
            let response = ApiError::validation(code, "evidence contract").into_response();
            assert_eq!(response.status(), expected, "{code} status mapping");
        }
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
    fn repository_registration_api_error_sanitizes_changed_paths_at_boundary() {
        let api_error = ApiError::from(RepositoryRegistrationError {
            stage: "repository_init_command".to_string(),
            provider: Some("claude_code".to_string()),
            command_index: Some(2),
            command: Some("/rule-config".to_string()),
            reason_code: "repository_init_command_failed".to_string(),
            stderr_summary: Some("safe diagnostic".to_string()),
            changed_paths: Some(vec![
                "/private/repo/generated".to_string(),
                ".claude/rules/project.md".to_string(),
                "src/monkey.rs".to_string(),
            ]),
            retryable: true,
            action: "Inspect changed paths and retry.".to_string(),
        });

        assert_eq!(
            api_error.details["changed_paths"],
            json!(["<path>", ".claude/rules/project.md", "src/monkey.rs"])
        );
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
