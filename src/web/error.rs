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
            "invalid_task_request" | "reviewer_window_decision_invalid" => StatusCode::BAD_REQUEST,
            "checkpoint_unsafe_dirty_worktree"
            | "workspace_session_ambiguous"
            | "workspace_session_confirm_not_allowed"
            | "workspace_session_takeover_not_allowed"
            | "workspace_choice_conflict"
            // C2 Task 12：gate response 同 command 异 payload 冲突（请刷新）。
            | "coding_gate_response_command_conflict"
            // P0 1.3（Task 9）：coding choice 应答同映射语义。
            | "coding_choice_conflict" => StatusCode::CONFLICT,
            // 批 2 T3（REQ-WIGA-09）：统一终止二次确认要求——未确认零副作用，
            // 428 Precondition Required（REST/gate-responses 共用同一码）。
            "automation_plan_termination_required" => StatusCode::PRECONDITION_REQUIRED,
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
            | "aggregate_initialization_operation_not_found"
            | "spec_not_found"
            | "workspace_not_found"
            | "work_item_not_found"
            | "task_workspace_not_found"
            | "workspace_session_not_found"
            | "registration_preflight_not_found"
            | "registration_batch_not_found"
            | "logical_codebase_not_found"
            | "workspace_choice_unknown"
            | "coding_choice_unknown" => StatusCode::NOT_FOUND,
            // P0 1.3：choice 应答旧 run/结构拒绝——410 Gone（不可送新 run）。
            "workspace_choice_expired" | "workspace_choice_rejected" => StatusCode::GONE,
            "coding_choice_expired" | "coding_choice_rejected" => StatusCode::GONE,
            "repository_project_not_found" | "repository_routing_target_unknown" => {
                StatusCode::NOT_FOUND
            }
            "workspace_session_review_not_enabled"
            | "repository_routing_target_missing"
            | "involved_repository_not_effective"
            | "change_order_repository_not_involved"
            | "change_order_duplicate_repository"
            | "mixed_target_group_rejected"
            | "target_snapshot_missing_for_logical"
            | "aggregate_index_unavailable"
            | "issue_selection_write_failed"
            | "issue_base_branch_required"
            | "issue_base_branch_not_found"
            | "repository_branch_list_failed"
            // REQ-MRE-01：创建链 focus 勾选集含 LC active 成员之外的 id → 422。
            | "focus_repository_outside_active_members" => StatusCode::UNPROCESSABLE_ENTITY,
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
            | "registration_batch_not_cancelable"
            // Bug3（2026-10-16）：LC restore 的两类可操作冲突——子树不完整 /
            // source 被重新登记占用，均 409（墓碑保留、可人工裁决后重试）。
            | "logical_codebase_restore_subtree_incomplete"
            | "logical_codebase_restore_source_taken"
            | "logical_codebase_name_conflict" => StatusCode::CONFLICT,
            "cross_target_baseline_missing" | "cross_target_store_failure" => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            "repository_routing_inconsistent"
            | "repository_routing_ambiguous"
            | "repository_routing_kind_mismatch"
            | "repository_routing_legacy_conflict"
            | "repository_routing_source_identity_mismatch" => {
                StatusCode::CONFLICT
            }
            "gate_ambiguous"
            | "invalid_execution_record_id"
            | "invalid_artifact_id"
            | "invalid_file_path"
            // Bug45/F4:file_diff 的 base_checkpoint 形态白名单拒绝 → 400。
            | "invalid_base_checkpoint"
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
            | "logical_codebase_name_required"
            | "aggregate_root_required"
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
            // 终止链（组删/N7 单删）reconcile 的远端探测失败/推送结果不可
            // 判定：可操作人工阻断（核对远端后重删/手动收敛），稳定 409。
            | "review_push_indeterminate"
            | "coding_workspace_exists"
            // 级联删除 D2.2（fix-cascade-deletion）：三类 DELETE 稳定 409 码——
            // 分级确认必答 / 存活引用拒删 / 同 scope 运行或资源 writer 竞态。
            // D2.3 族分派补全：删除窗口迟到写（with_live_write）与已删身份
            //（身份准入/反查）同族 409，不再经 product_store_error 默认 500。
            | "deletion_confirmation_required"
            | "entity_referenced"
            | "entity_busy"
            | "entity_deleting"
            | "entity_deleted"
            | "aggregate_root_ownership_conflict" => StatusCode::CONFLICT,
            // 级联删除 D2.3：N2 冻结错误族（fix-multi-repo-group-coding-start
            // REQ-CS-02/05/07）REST 面直达——冻结身份/revision/graph/相位/集合/
            // selection 冲突 409（post_plan_coding_start 等 product_store_error
            // 直通 kind）；provider 准入边界（安全暂停/失效/完成窗口）409。
            | "coding_plan_run_revision"
            | "coding_plan_run_plan_revision"
            | "coding_plan_run_graph"
            | "coding_plan_run_replay_drift"
            | "coding_plan_run_phase"
            | "coding_plan_run_not_active"
            | "coding_plan_run_invalidated"
            | "coding_plan_run_set_not_first_start"
            | "coding_plan_run_outcome_frozen"
            | "coding_plan_run_selection"
            | "coding_plan_run_selection_attempt"
            | "coding_plan_run_selection_target"
            | "coding_plan_run_selection_unit"
            | "coding_plan_run_selection_not_ready"
            | "coding_plan_run_start_command"
            | "coding_plan_run_attempt_binding"
            | "coding_plan_run_attempt_target"
            | "coding_plan_run_target_attempts"
            | "coding_plan_run_selected_unit"
            | "plan_coding_paused"
            | "plan_coding_invalidated"
            // H4（设计 2026-10-14 B 节,H2a 码透出 + H5 移交④清理中/资源
            // busy 稳定码):执行轮生命周期状态冲突与目标仓资源占用统一
            // 409——clearing 续清/退役围栏/generation 互证/成员在场/资源
            // busy 或删除中,均为可重试或可收敛的产品状态,非 500 故障。
            | "coding_plan_workspace_clearing"
            | "coding_plan_workspace_generation_conflict"
            | "coding_plan_workspace_members_present"
            | "plan_coding_run_resource_busy"
            | "plan_coding_run_resource_unavailable" => StatusCode::CONFLICT,
            // N2 族语义拒绝：空 selection 不能首启 / purge 作用域非法 / 在途
            // amendment / plan 未确认（对齐 work_item_plan_not_confirmed 的
            // 前置语义，REST 前 ensure_plan_confirmed 已先拒，此处为防御层）。
            | "coding_plan_run_selection_missing"
            | "coding_plan_run_purge_scope"
            | "coding_plan_run_active_amendment"
            | "coding_plan_run_plan_not_confirmed" => StatusCode::UNPROCESSABLE_ENTITY,
            // P1 修复轮：族内未逐一登记的 kind（如后续新增）按前缀默认 409
            //（run 冻结状态拒绝），显式 422 清单优先——不再落 500。
            // P2 修复轮：`coding_plan_run_target` 除外——repo_lease_state 的
            // 完整性错误（target 非法 UUID），owner 设计 §B「真实 IO/JSON/
            // 损坏身份不是 409」，保留 500 故障信号。
            c if c.starts_with("coding_plan_run_")
                && c != "coding_plan_run_target" =>
            {
                StatusCode::CONFLICT
            }
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
            // WIG Autopilot P0 enrollment 稳定码：冲突 409、缺失 404、作用域 422。
            "automation_enrollment_conflict" => StatusCode::CONFLICT,
            "automation_enrollment_not_found" => StatusCode::NOT_FOUND,
            "automation_enrollment_invalid_scope" => StatusCode::UNPROCESSABLE_ENTITY,
            "automation_role_chain_unsupported" => StatusCode::UNPROCESSABLE_ENTITY,
            // 批 2 P4（REQ-WIGA-10）：plan 继续/control 稳定码——终止/忙/绑定
            // 漂移/停点键不符/命令账本冲突 409；作用域与形状拒绝 422。
            "automation_plan_terminated"
            | "automation_continue_busy"
            | "automation_continue_binding_mismatch"
            | "automation_continue_stop_key_mismatch"
            | "automation_continue_command_conflict"
            | "automation_continue_gate_reopen_conflict" => StatusCode::CONFLICT,
            "automation_continue_rejected"
            | "automation_continue_not_stopped"
            | "automation_continue_context_required" => StatusCode::UNPROCESSABLE_ENTITY,
            // C5 Task 6（REQ-INIT-C5-RESUME）：resume 前置校验稳定码——
            // 非 Failed 终态与非法 command_id 均 422（不创建 successor）。
            "repository_initialization_resume_not_failed" => StatusCode::UNPROCESSABLE_ENTITY,
            "repository_initialization_resume_invalid_command" => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            // P0 1.3（Task 10）human-actions 稳定码：门 id 不匹配 409、空白
            // command_id 400、引擎语义拒绝/compile recovery 越节点/approve
            // compile 失败 422（失败不宣称 Confirmed）。
            "human_action_gate_mismatch" => StatusCode::CONFLICT,
            "invalid_command_id" => StatusCode::BAD_REQUEST,
            "human_action_rejected" => StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_compile_recovery_action" => StatusCode::UNPROCESSABLE_ENTITY,
            "single_candidate_approval_compile_failed" => StatusCode::UNPROCESSABLE_ENTITY,
            // T5.1（REQ-REG-15/D4）：确认 trust 未 Ready 的停等 422——语义
            // fail-closed（门未关、不宣称 Confirmed），非 500 服务错误。
            "workspace_confirmation_trust_waiting" => StatusCode::UNPROCESSABLE_ENTITY,
            // C1 Task 4：候选门恢复/缺快照裸 approve 稳定码——门/命令
            // 身份冲突 409，语义拒绝/快照不完整 422。
            "candidate_recovery_gate_mismatch" => StatusCode::CONFLICT,
            "candidate_recovery_command_conflict" => StatusCode::CONFLICT,
            "candidate_recovery_rejected" => StatusCode::UNPROCESSABLE_ENTITY,
            "candidate_snapshot_incomplete" => StatusCode::UNPROCESSABLE_ENTITY,
            // C1 Task 6/7：lease 确认接管与 Failed advance 显式 retry 稳定码
            // ——身份/命令冲突 409、参数非法 400、会话缺失 404、语义拒绝
            //（过期 checkpoint/binding/副作用门前的引擎拒绝）422。
            "lease_takeover_identity_mismatch" => StatusCode::CONFLICT,
            "lease_takeover_command_conflict" => StatusCode::CONFLICT,
            "retry_initialization_invalid_id" => StatusCode::BAD_REQUEST,
            "retry_initialization_session_not_found" => StatusCode::NOT_FOUND,
            "retry_initialization_rejected" => StatusCode::UNPROCESSABLE_ENTITY,
            // P2 GAP-E/G（Task 0.1）failed-sc-runs retry 稳定码：前置
            // 不满足 409（busy/非最新失败节点/认领异键）、请求非法 422。
            "sc_recovery_busy" => StatusCode::CONFLICT,
            "sc_recovery_node_mismatch" => StatusCode::CONFLICT,
            "sc_recovery_conflict" => StatusCode::CONFLICT,
            "sc_recovery_rejected" => StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_pointer_request" => StatusCode::UNPROCESSABLE_ENTITY,
            // C4 Task 6：bootstrap 动作稳定码——过期 revision/object 冲突 409，
            // 请求参数非法 400/422。
            "bootstrap_stale_revision" => StatusCode::CONFLICT,
            "bootstrap_action_invalid_object" => StatusCode::BAD_REQUEST,
            // C4 Task 7：identity repair 稳定码——journal 缺失 404、
            // stale journal/未决冲突 409、mapping 语义拒绝/参数非法 422。
            "identity_repair_journal_not_found" => StatusCode::NOT_FOUND,
            "identity_repair_stale_journal"
            | "identity_repair_conflict"
            | "identity_repair_not_failed" => StatusCode::CONFLICT,
            "identity_repair_rejected"
            | "identity_repair_invalid_action"
            | "identity_repair_invalid_mapping"
            | "identity_repair_mapping_required"
            | "identity_repair_mapping_unexpected" => StatusCode::UNPROCESSABLE_ENTITY,
            // P3 WIGA 有界近期完成查询稳定码：缺 since、非 RFC3339、limit
            // 越界均 422（ApiError::validation 不自行决定状态，须显式映射）。
            "recent_since_required" | "invalid_recent_since" | "invalid_recent_limit" => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            // add-provider-revalidate-probe：Fake/未知 provider 422（不冒充
            // 探测）；探针失败关闭 503（环境侧真实失败，可重试，不伪造
            // Confirmed）。
            "provider_capability_probe_unsupported" => StatusCode::UNPROCESSABLE_ENTITY,
            "provider_capability_probe_failed" => StatusCode::SERVICE_UNAVAILABLE,
            // Task 7 证据查询稳定码：6 码 + evidence_io（设计 §5.2）。
            "evidence_unauthorized" => StatusCode::UNAUTHORIZED,
            "evidence_forbidden" => StatusCode::FORBIDDEN,
            "evidence_not_available" => StatusCode::NOT_FOUND,
            "evidence_invalid_query" => StatusCode::UNPROCESSABLE_ENTITY,
            "evidence_budget_exhausted" => StatusCode::TOO_MANY_REQUESTS,
            "evidence_query_failed" => StatusCode::SERVICE_UNAVAILABLE,
            "evidence_io" => StatusCode::INTERNAL_SERVER_ERROR,
            // C2 oracle C-2：验证处理/重跑 REST 面引擎 reason code 直达——
            // 验证处理 fail-closed 拒绝（422）；重跑版本/门身份过期"请刷新"
            //（409）；其余 coding_rerun_*（不可满足 check 等）422。
            c if c.starts_with("verification_triage_")
                || c == "coding_rerun_gate_not_eligible"
                || c == "coding_rerun_check_has_no_command" => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            "coding_rerun_version_conflict" | "coding_rerun_gate_not_open" => {
                StatusCode::CONFLICT
            }
            // C2 oracle C-2c：受限政策读取/重新授权稳定码（PolicyAccessError
            // 直达；resolver fail-closed 停等 503，拒绝链 401/403/404/409）。
            "policy_unauthorized" => StatusCode::UNAUTHORIZED,
            "policy_forbidden" => StatusCode::FORBIDDEN,
            "policy_not_available" => StatusCode::NOT_FOUND,
            "policy_command_conflict" | "policy_version_conflict" => StatusCode::CONFLICT,
            "policy_resolver_unavailable" | "policy_digest_mismatch" | "policy_read_failed" => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            // C2 oracle C-2a：验证处理记录缺失（决定面）。
            "coding_verification_triage_not_found" => StatusCode::NOT_FOUND,
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
        let error = error.into_details();
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
    use crate::product::repository_store::{
        RepositoryRegistrationError, RepositoryRegistrationErrorDetails,
    };

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
            ("repository_routing_kind_mismatch", StatusCode::CONFLICT),
            ("repository_routing_legacy_conflict", StatusCode::CONFLICT),
            (
                "repository_routing_source_identity_mismatch",
                StatusCode::CONFLICT,
            ),
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
            let api_error = ApiError::from(RepositoryRegistrationError::from(
                RepositoryRegistrationErrorDetails {
                    stage: "repository_init_command".to_string(),
                    provider: Some("claude_code".to_string()),
                    command_index: Some(2),
                    command: Some("/rule-config".to_string()),
                    reason_code: code.to_string(),
                    stderr_summary: Some("safe diagnostic".to_string()),
                    changed_paths: Some(vec![".claude/rules/project.md".to_string()]),
                    retryable: true,
                    action:
                        "Fix the problem, inspect changed_paths, then add the repository again."
                            .to_string(),
                },
            ));

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
        let api_error = ApiError::from(RepositoryRegistrationError::from(
            RepositoryRegistrationErrorDetails {
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
            },
        ));

        assert_eq!(
            api_error.details["changed_paths"],
            json!(["<path>", ".claude/rules/project.md", "src/monkey.rs"])
        );
    }

    #[test]
    fn repository_registration_api_error_uses_nulls_and_empty_changed_paths() {
        let api_error = ApiError::from(RepositoryRegistrationError::from(
            RepositoryRegistrationErrorDetails {
                stage: "repository_path".to_string(),
                provider: None,
                command_index: None,
                command: None,
                reason_code: "repository_path_invalid".to_string(),
                stderr_summary: None,
                changed_paths: None,
                retryable: false,
                action: "Choose a valid path, then add the repository again.".to_string(),
            },
        ));

        assert_eq!(api_error.details["provider"], Value::Null);
        assert_eq!(api_error.details["command"], Value::Null);
        assert_eq!(api_error.details["stderr_summary"], Value::Null);
        assert_eq!(api_error.details["changed_paths"], json!([]));
    }

    #[test]
    fn repository_registration_api_error_sanitizes_public_summary() {
        let api_error = ApiError::from(RepositoryRegistrationError::from(
            RepositoryRegistrationErrorDetails {
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
            },
        ));
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
    #[test]
    fn cascade_d2_3_entity_family_maps_to_stable_statuses() {
        // 级联删除 D2.3（fix-cascade-deletion REQ-DEL-04/06）：删除窗口迟到写
        //（with_live_write entity_deleting）与已删身份（entity_deleted）必须
        // 稳定 409，不得经 product_store_error 默认 500 泄漏；三类 DELETE 的
        // 404 缺口（无墓碑无 root）由 spec_not_found 收口。
        let entity_cases = [
            ("entity_deleting", StatusCode::CONFLICT),
            ("entity_deleted", StatusCode::CONFLICT),
            ("spec_not_found", StatusCode::NOT_FOUND),
        ];
        for (code, expected_status) in entity_cases {
            let response = ApiError::runtime(code, "cascade family", json!({})).into_response();
            assert_eq!(response.status(), expected_status, "{code} status mapping");
        }
    }

    #[test]
    fn cascade_d2_3_plan_coding_run_family_maps_to_stable_statuses() {
        // N2 冻结错误族（fix-multi-repo-group-coding-start REQ-CS-02/05/07）经
        // REST 面（post_plan_coding_start 等 product_store_api_error）直达：
        // 冻结身份/相位/集合冲突 409；语义拒绝（空首启/作用域/在途 amendment）
        // 422；未确认前置 400（对齐 work_item_plan_not_confirmed）。只做已冻结
        // 的映射面，不改 N2 语义（重启语义归 DesignDelRestart 在途设计）。
        let conflict_cases = [
            "coding_plan_run_revision",
            "coding_plan_run_plan_revision",
            "coding_plan_run_graph",
            "coding_plan_run_replay_drift",
            "coding_plan_run_phase",
            "coding_plan_run_not_active",
            "coding_plan_run_invalidated",
            "coding_plan_run_set_not_first_start",
            "coding_plan_run_outcome_frozen",
            "coding_plan_run_selection",
            "coding_plan_run_selection_attempt",
            "coding_plan_run_selection_target",
            "coding_plan_run_selection_unit",
            "coding_plan_run_selection_not_ready",
            "coding_plan_run_start_command",
            "coding_plan_run_attempt_binding",
            "coding_plan_run_attempt_target",
            "coding_plan_run_target_attempts",
            "coding_plan_run_selected_unit",
            "plan_coding_paused",
            "plan_coding_invalidated",
        ];
        for code in conflict_cases {
            let response = ApiError::runtime(code, "plan coding run", json!({})).into_response();
            assert_eq!(
                response.status(),
                StatusCode::CONFLICT,
                "{code} status mapping"
            );
        }
        let rejected_cases = [
            "coding_plan_run_selection_missing",
            "coding_plan_run_purge_scope",
            "coding_plan_run_active_amendment",
            "coding_plan_run_plan_not_confirmed",
        ];
        for code in rejected_cases {
            let response = ApiError::runtime(code, "plan coding run", json!({})).into_response();
            assert_eq!(
                response.status(),
                StatusCode::UNPROCESSABLE_ENTITY,
                "{code} status mapping"
            );
        }
        // P1 修复轮：族内未逐一登记 kind 按前缀默认 409（显式 422 清单优先）。
        let fallback_response =
            ApiError::runtime("coding_plan_run_future_kind", "plan coding run", json!({}))
                .into_response();
        assert_eq!(
            fallback_response.status(),
            StatusCode::CONFLICT,
            "unlisted coding_plan_run_ kind prefix fallback"
        );
        // P2 修复轮（Main 审查派回）：`coding_plan_run_target` 是 repo_lease_state
        // 的完整性错误（target repository id 非法 UUID，InvalidRecord）——owner
        // 设计 §B「真实 IO/JSON/损坏身份不是 409」，前缀兜底须排除、保留 500。
        let target_response = ApiError::runtime(
            "coding_plan_run_target",
            "target repository id is not a logical UUID",
            json!({}),
        )
        .into_response();
        assert_eq!(
            target_response.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "coding_plan_run_target must stay 500 (integrity signal)"
        );
    }
}
