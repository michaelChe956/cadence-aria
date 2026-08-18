pub(crate) fn logical_codebase_feature_disabled_api_error(project_id: &str) -> ApiError {
    ApiError::runtime(
        "logical_codebase_feature_disabled",
        "logical codebase features are disabled for single-repository projects",
        json!({ "project_id": project_id }),
    )
}

pub(crate) fn legacy_repository_endpoint_on_multi_repo_api_error(
    project_id: &str,
) -> ApiError {
    ApiError::runtime(
        "legacy_repository_endpoint_on_multi_repo",
        "legacy repository endpoint is unavailable for multi-repository projects; 请使用逻辑代码库登记端点",
        json!({ "project_id": project_id }),
    )
}

fn repository_routing_api_error(
    code: &'static str,
    message: &'static str,
    details: serde_json::Value,
) -> ApiError {
    ApiError::runtime(code, message, details)
}

/// RepositoryRouting FailClosed 的稳定错误码映射。
pub(crate) fn routing_api_error(code: RepositoryRoutingErrorCode, reason: &str) -> ApiError {
    let stable_code = code.stable_code();
    ApiError::runtime(
        stable_code,
        "repository routing failed closed",
        json!({ "reason": reason }),
    )
}

fn routing_error_code_from_reason(kind: &str, reason: &str) -> Option<&'static str> {
    if kind == "repository_routing" {
        return [
            "repository_routing_target_missing",
            "repository_routing_inconsistent",
            "repository_routing_target_unknown",
            "repository_routing_ambiguous",
        ]
        .into_iter()
        .find(|code| {
            reason == *code
                || reason
                    .strip_prefix(code)
                    .is_some_and(|suffix| suffix.starts_with(": "))
        });
    }

    match (kind, reason) {
        ("effective_member_empty", reason)
            if reason.ends_with(": no effective member; primary fallback forbidden") =>
        {
            Some("repository_routing_inconsistent")
        }
        ("issue_codebase_selection", "member_removed") => Some("repository_routing_inconsistent"),
        ("issue_codebase_selection", "selection_invalidated") => {
            Some("repository_routing_inconsistent")
        }
        _ => None,
    }
}

pub(crate) fn product_store_api_error(error: ProductStoreError) -> ApiError {
    match error {
        ProductStoreError::NotFound {
            kind: "project", ..
        } => ApiError::runtime("project_not_found", "project not found", json!({})),
        ProductStoreError::NotFound {
            kind: "issue_codebase_selection",
            id,
        } => repository_routing_api_error(
            "repository_routing_target_missing",
            "issue codebase selection is required for repository routing",
            json!({"kind": "issue_codebase_selection", "id": id}),
        ),
        ProductStoreError::NotFound {
            kind: kind @ ("logical_repository" | "identity_resolution_missing"),
            id,
        } => repository_routing_api_error(
            "repository_routing_target_unknown",
            "repository routing target is unknown",
            json!({"kind": kind, "id": id}),
        ),
        ProductStoreError::NotFound {
            kind: kind @ ("logical_repository_manifest" | "logical_codebase_manifest"),
            id,
        } => repository_routing_api_error(
            "repository_routing_inconsistent",
            "repository routing authority is inconsistent",
            json!({"kind": kind, "id": id}),
        ),
        ProductStoreError::NotFound {
            kind: kind @ ("logical_codebase_member" | "repository_checkout"),
            id,
        } => repository_routing_api_error(
            "repository_routing_target_unknown",
            "repository routing target is unknown",
            json!({"kind": kind, "id": id}),
        ),
        ProductStoreError::Ambiguous {
            kind:
                kind @ ("issue_codebase_selection"
                | "logical_repository"
                | "identity_resolution_ambiguous"),
            id,
        } => repository_routing_api_error(
            "repository_routing_ambiguous",
            "repository routing target is ambiguous",
            json!({"kind": kind, "id": id}),
        ),
        ProductStoreError::Conflict {
            kind: kind @ ("issue_codebase_selection" | "logical_repository"),
            id,
        }
        | ProductStoreError::IdentityMismatch {
            kind:
                kind @ ("issue_codebase_selection"
                | "logical_repository"
                | "logical_repository_resolution"
                | "logical_codebase_manifest"
                | "repository_projection"),
            id,
        } => repository_routing_api_error(
            "repository_routing_inconsistent",
            "repository routing authority is inconsistent",
            json!({"kind": kind, "id": id}),
        ),
        ProductStoreError::InvalidRecord { kind, reason } => {
            if let Some(code) = routing_error_code_from_reason(kind, &reason) {
                repository_routing_api_error(
                    code,
                    "repository routing failed closed",
                    json!({"kind": kind, "reason": reason}),
                )
            } else {
                ApiError::runtime(
                    "product_store_error",
                    "product store operation failed",
                    json!({"kind": kind, "reason": reason}),
                )
            }
        }
        ProductStoreError::NotFound {
            kind: "repository", ..
        } => ApiError::runtime("repository_not_found", "repository not found", json!({})),
        ProductStoreError::NotFound {
            kind: "repository_initialization_operation",
            ..
        } => ApiError::runtime(
            "repository_initialization_operation_not_found",
            "repository initialization operation not found",
            json!({}),
        ),
        ProductStoreError::NotFound { kind: "issue", .. } => {
            ApiError::runtime("issue_not_found", "issue not found", json!({}))
        }
        ProductStoreError::NotFound {
            kind: "work_item", ..
        } => ApiError::runtime("work_item_not_found", "work item not found", json!({})),
        ProductStoreError::NotFound {
            kind: "coding_attempt",
            ..
        } => ApiError::runtime(
            "coding_attempt_not_found",
            "coding attempt not found",
            json!({}),
        ),
        ProductStoreError::NotFound {
            kind: "workspace_session",
            ..
        } => ApiError::runtime(
            "workspace_session_not_found",
            "workspace session not found",
            json!({}),
        ),
        ProductStoreError::NotFound { kind: "gate", .. } => {
            ApiError::runtime("gate_not_found", "gate not found", json!({}))
        }
        ProductStoreError::Io(message) if message == "mixed_target_group_rejected" => {
            ApiError::runtime(
                "mixed_target_group_rejected",
                "mixed-target group rejected",
                json!({}),
            )
        }
        ProductStoreError::Io(message) if message == "workspace_session_ambiguous" => {
            ApiError::runtime(
                "workspace_session_ambiguous",
                "workspace session matches multiple files",
                json!({}),
            )
        }
        ProductStoreError::Io(message) if message == "gate_ambiguous" => ApiError::runtime(
            "gate_ambiguous",
            "gate matches multiple projects",
            json!({}),
        ),
        ProductStoreError::Ambiguous {
            kind: "coding_attempt",
            id,
        } => ApiError::runtime(
            "coding_attempt_ambiguous",
            "coding attempt matches multiple issues",
            json!({"attempt_id": id}),
        ),
        ProductStoreError::Conflict {
            kind: "logical_codebase_feature_disabled",
            id,
        } => logical_codebase_feature_disabled_api_error(&id),
        ProductStoreError::Conflict {
            kind: "active_coding_attempt",
            id,
        } => ApiError::runtime(
            "coding_attempt_active",
            "an active coding attempt already exists for this work item",
            json!({"attempt_id": id}),
        ),
        ProductStoreError::IdentityMismatch {
            kind: "coding_attempt",
            id,
        } => ApiError::runtime(
            "coding_attempt_scope_mismatch",
            "coding attempt does not belong to the requested project and issue",
            json!({"attempt_id": id}),
        ),
        ProductStoreError::PathEscape(_) => {
            ApiError::validation("invalid_project_id", "invalid project id")
        }
        other => {
            let details = match &other {
                ProductStoreError::NotFound { kind, id }
                | ProductStoreError::Ambiguous { kind, id }
                | ProductStoreError::Conflict { kind, id }
                | ProductStoreError::IdentityMismatch { kind, id } => {
                    json!({ "kind": kind, "id": id })
                }
                ProductStoreError::InvalidRecord { kind, reason } => {
                    json!({ "kind": kind, "reason": reason })
                }
                ProductStoreError::Io(message)
                | ProductStoreError::Json(message)
                | ProductStoreError::PathEscape(message) => json!({ "message": message }),
            };
            ApiError::runtime(
                "product_store_error",
                "product store operation failed",
                details,
            )
        }
    }
}
