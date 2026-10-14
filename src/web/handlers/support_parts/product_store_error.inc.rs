pub(crate) fn issue_selection_write_failed_api_error() -> ApiError {
    ApiError::runtime(
        "issue_selection_write_failed",
        "issue codebase selection could not be saved",
        json!({}),
    )
}

pub(crate) fn aggregate_root_api_error(
    error: crate::product::logical_codebase::AggregateRootPreflightError,
) -> ApiError {
    let code = match error.code() {
        "aggregate_root_is_git" => "aggregate_root_is_git",
        "member_path_outside_root" => "aggregate_root_member_outside_root",
        "member_symlink_escape" => "aggregate_root_member_symlink_escape",
        "nested_worktree" => "aggregate_root_nested_worktree",
        "aggregate_root_ownership_conflict" | "aggregate_root_overlap" => {
            "aggregate_root_ownership_conflict"
        }
        "aggregate_root_missing" => "aggregate_root_missing",
        "aggregate_root_invalid_project_id" => "invalid_project_id",
        _ => "aggregate_root_internal_error",
    };
    ApiError::runtime(
        code,
        "aggregate root preflight rejected",
        json!({ "reason": error.message() }),
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
            "repository_routing_kind_mismatch",
            "repository_routing_legacy_conflict",
            "repository_routing_source_identity_mismatch",
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
        ProductStoreError::Conflict { kind, id }
            if matches!(
                kind,
                "repository_routing_kind_mismatch"
                    | "repository_routing_legacy_conflict"
                    | "repository_routing_source_identity_mismatch"
            ) =>
        {
            ApiError::runtime(
                kind,
                "repository routing failed closed",
                json!({"kind": kind, "id": id}),
            )
        }
        ProductStoreError::NotFound {
            kind: "registration_preflight",
            ..
        } => ApiError::runtime(
            "registration_preflight_not_found",
            "registration preflight not found or expired",
            json!({}),
        ),
        // C2 oracle C-2a：验证处理记录缺失（决定面）→ 稳定 404 码。
        ProductStoreError::NotFound {
            kind: "coding_verification_triage",
            id,
        } => ApiError::runtime(
            "coding_verification_triage_not_found",
            "verification triage record not found",
            json!({ "id": id }),
        ),
        ProductStoreError::Conflict {
            kind: "registration_batch_candidate_identity_changed",
            ..
        }
        | ProductStoreError::IdentityMismatch {
            kind: "registration_batch_member_recovery",
            ..
        } => ApiError::runtime(
            "registration_batch_conflict",
            "registration batch conflicts with current repository identity",
            json!({}),
        ),
        ProductStoreError::Conflict {
            kind: "aggregate_root_mismatch",
            ..
        } => ApiError::runtime(
            "aggregate_root_mismatch",
            "aggregate root does not match the logical codebase manifest",
            json!({}),
        ),
        ProductStoreError::Conflict {
            kind: "aggregate_initialization",
            id,
        } => ApiError::runtime(
            "aggregate_initialization_conflict",
            "aggregate initialization operation conflicts with an existing idempotency key",
            json!({"operation_id": id}),
        ),
        ProductStoreError::NotFound {
            kind: "logical_codebase",
            ..
        } => ApiError::runtime(
            "logical_codebase_not_found",
            "logical codebase not found",
            json!({}),
        ),
        ProductStoreError::Conflict {
            kind: "logical_codebase_name",
            id,
        } => ApiError::runtime(
            "logical_codebase_name_conflict",
            "logical codebase name already exists in this project",
            json!({ "name": id }),
        ),
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
        // 级联删除 D2.3 P1 修复轮（Main 审查派回）：N2 冻结错误族跨三变体生产
        //——InvalidRecord（plan_revision/selection_unit）前缀直通稳定码，
        // details 保留 kind/reason；须先于下方 routing InvalidRecord 臂匹配。
        // P2 修复轮：`coding_plan_run_target` 除外——repo_lease_state 完整性
        // 错误（target 非法 UUID），落 other 兜底 500 保留 {kind,reason} 信号。
        ProductStoreError::InvalidRecord { kind, reason }
            if kind.starts_with("coding_plan_run_") && kind != "coding_plan_run_target" =>
        {
            ApiError::runtime(
                kind,
                "plan coding run 冻结记录拒绝该操作",
                json!({ "kind": kind, "reason": reason }),
            )
        }
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
        // 级联删除 D2.3：删除窗口迟到写（with_live_write）与已删身份直通稳定
        // 409 码（error.rs 族分派），不再落 product_store_error 500。
        ProductStoreError::Conflict {
            kind: "entity_deleting",
            id,
        } => ApiError::runtime(
            "entity_deleting",
            "实体删除进行中，该操作已被拒绝，请稍后重试",
            json!({ "kind": "entity_deleting", "id": id }),
        ),
        ProductStoreError::Conflict {
            kind: "entity_deleted",
            id,
        } => ApiError::runtime(
            "entity_deleted",
            "实体已删除，操作对象不复存在",
            json!({ "kind": "entity_deleted", "id": id }),
        ),
        // N2 冻结错误族（fix-multi-repo-group-coding-start REQ-CS-02/05/07）：
        // coding_plan_run_* Conflict kind 直通稳定码，details 保留 kind/id
        //（plan_start_tests 以 details.kind 断言）。只做映射面，不改语义。
        ProductStoreError::Conflict { kind, id } if kind.starts_with("coding_plan_run_") => {
            ApiError::runtime(
                kind,
                "plan coding run 冻结状态拒绝该操作",
                json!({ "kind": kind, "id": id }),
            )
        }
        // P1 修复轮续：IdentityMismatch 变体（revision/graph/replay_drift/
        // selection 族/attempt_binding 族/target_attempts/purge_scope 等 14 码）
        // 同前缀直通，details 保留 kind/id。
        ProductStoreError::IdentityMismatch { kind, id }
            if kind.starts_with("coding_plan_run_") =>
        {
            ApiError::runtime(
                kind,
                "plan coding run 冻结身份拒绝该操作",
                json!({ "kind": kind, "id": id }),
            )
        }
        // N2 provider 准入边界（gate.rs Io 形态 `plan_coding_*: {attempt_id}`）：
        // 安全暂停/失效拒绝 provider 进入，直达 409 稳定码。
        ProductStoreError::Io(message)
            if message.starts_with("plan_coding_paused:")
                || message.starts_with("plan_coding_invalidated:") =>
        {
            let (code, attempt_id) = message.split_once(':').unwrap_or((message.as_str(), ""));
            ApiError::runtime(
                code,
                "plan coding 安全暂停边界拒绝 provider 进入",
                json!({ "attempt_id": attempt_id.trim() }),
            )
        }
        // 级联删除 D2.3 裁定（D2.2 登记 1 的 Web 侧收口）：无墓碑无 root 的
        // Story/Design DELETE 在 classify 命中 NotFound{spec}——按 plan D2.2
        // Step 3「无墓碑无root404」映射 spec_not_found（error.rs 既有 404）。
        ProductStoreError::NotFound { kind: "spec", id } => {
            ApiError::runtime("spec_not_found", "spec not found", json!({ "id": id }))
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
