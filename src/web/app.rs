use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use tokio::net::TcpListener;

use crate::product::app_paths::ProductAppPaths;
use crate::product::product_data_schema::ensure_product_data_schema;
use crate::web::coding_ws_handler;
use crate::web::events::EventHub;
use crate::web::handlers;
use crate::web::state::WebAppState;
use crate::web::test_controls;
use crate::web::workspace_ws_handler;

pub fn build_web_router(state: WebAppState) -> Router {
    build_web_router_with_evidence(state, true)
}

pub fn build_web_router_with_evidence(state: WebAppState, evidence_enabled: bool) -> Router {
    let router = Router::new()
        .route("/api/health", get(handlers::health))
        .route(
            "/api/auth/session",
            get(crate::web::api_auth::session_status).post(crate::web::api_auth::create_session),
        )
        .route("/api/providers/status", get(handlers::providers_status))
        .route("/api/providers/recheck", post(handlers::providers_recheck))
        .route("/api/runtime-info", get(handlers::runtime_info))
        .route(
            "/api/image-create/sessions",
            get(handlers::list_image_create_sessions).post(handlers::create_image_create_session),
        )
        .route(
            "/api/image-create/sessions/{id}",
            get(handlers::get_image_create_session).delete(handlers::delete_image_create_session),
        )
        .route(
            "/api/image-create/sessions/{id}/images/{image_id}",
            get(handlers::image_create::get_session_image),
        )
        .route(
            "/api/image-create/sessions/{id}/chat",
            get(handlers::image_create_chat_ws),
        )
        .route(
            "/api/image-create/sessions/{id}/generate",
            post(handlers::generate_image)
                .layer(DefaultBodyLimit::max(11 * 1024 * 1024)),
        )
        .route(
            "/api/image-create/settings",
            get(handlers::get_image_create_settings).put(handlers::update_image_create_settings),
        )
        .route("/api/events", get(handlers::events))
        .route("/api/projection", get(handlers::projection))
        .route(
            "/api/tasks",
            get(handlers::list_tasks).post(handlers::create_task),
        )
        .route(
            "/api/workspaces",
            get(handlers::list_workspaces).post(handlers::create_workspace),
        )
        .route(
            "/api/workspaces/{workspace_id}",
            delete(handlers::delete_workspace),
        )
        .route(
            "/api/projects",
            get(handlers::list_projects).post(handlers::create_project),
        )
        .route(
            "/api/projects/{project_id}",
            get(handlers::get_project).delete(handlers::delete_project),
        )
        .route(
            "/api/projects/{project_id}/open",
            post(handlers::open_project),
        )
        .route(
            "/api/projects/{project_id}/repositories",
            get(handlers::list_repositories).post(handlers::create_repository),
        )
        .route(
            "/api/projects/{project_id}/repositories/{repository_id}",
            delete(handlers::delete_repository),
        )
        .route(
            "/api/projects/{project_id}/repositories/{repository_id}/branches",
            get(handlers::list_repository_branches),
        )
        .route(
            "/api/projects/{project_id}/repository-initializations/{operation_id}",
            get(handlers::get_repository_initialization),
        )
        .route(
            "/api/projects/{project_id}/repository-initializations/waiting-items",
            get(handlers::get_project_repository_initialization_waiting_items),
        )
        .route(
            "/api/projects/{project_id}/repository-initializations/{operation_id}/resume",
            post(handlers::post_repository_initialization_resume),
        )
        .route(
            "/api/projects/{project_id}/codebases",
            get(handlers::list_codebases),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases",
            post(handlers::create_logical_codebase),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{logical_codebase_id}",
            get(handlers::get_logical_codebase).delete(handlers::delete_logical_codebase),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{logical_codebase_id}/restore",
            post(handlers::restore_logical_codebase),
        )
        .route(
            "/api/projects/{project_id}/logical-codebase/initializations",
            post(handlers::create_aggregate_initialization),
        )
        .route(
            "/api/projects/{project_id}/logical-codebase/initializations/active",
            get(handlers::get_aggregate_initialization_active),
        )
        .route(
            "/api/projects/{project_id}/logical-codebase/initializations/{operation_id}",
            get(handlers::get_aggregate_initialization),
        )
        .route(
            "/api/projects/{project_id}/logical-codebase/initializations/{operation_id}/cancel",
            post(handlers::cancel_aggregate_initialization),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/initializations",
            post(handlers::create_lc_aggregate_initialization),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/initializations/active",
            get(handlers::get_lc_aggregate_initialization_active),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/initializations/{operation_id}",
            get(handlers::get_lc_aggregate_initialization),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/initializations/{operation_id}/cancel",
            post(handlers::cancel_lc_aggregate_initialization),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/bootstrap",
            get(handlers::get_lc_bootstrap_projection),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/bootstrap/actions",
            post(handlers::post_lc_bootstrap_action),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/capabilities",
            get(handlers::get_lc_capabilities),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/capability-revalidate",
            post(handlers::post_lc_capability_revalidate),
        )
        .route(
            "/api/projects/{project_id}/logical-codebase/bootstrap",
            get(handlers::get_logical_codebase_bootstrap),
        )
        .route(
            "/api/projects/{project_id}/logical-codebase/bootstrap/actions",
            post(handlers::post_logical_codebase_bootstrap_action),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/identity-repair",
            get(handlers::get_lc_identity_repair)
                .post(handlers::post_lc_identity_repair_action),
        )
        .route(
            "/api/projects/{project_id}/logical-codebase/members",
            get(handlers::list_logical_codebase_members),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/members",
            get(handlers::list_lc_logical_codebase_members),
        )
        .route(
            "/api/projects/{project_id}/logical-codebase/aggregate-indexes/active",
            get(handlers::get_active_aggregate_index),
        )
        .route(
            "/api/projects/{project_id}/logical-codebase/aggregate-indexes/rebuild",
            post(handlers::rebuild_aggregate_index),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/aggregate-indexes/active",
            get(handlers::get_lc_active_aggregate_index),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/aggregate-indexes/rebuild",
            post(handlers::rebuild_lc_aggregate_index),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/registrations/preflight",
            post(handlers::preflight_lc_registration),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/registrations",
            post(handlers::submit_lc_registration),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/registrations/{batch_id}",
            get(handlers::get_lc_registration_batch),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/registrations/{batch_id}/resume",
            post(handlers::resume_lc_registration),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/registrations/{batch_id}/cancel",
            post(handlers::cancel_lc_registration),
        )
        .route(
            "/api/projects/{project_id}/logical-codebase/registrations/preflight",
            post(handlers::preflight_logical_codebase_registration),
        )
        .route(
            "/api/projects/{project_id}/logical-codebase/registrations",
            post(handlers::submit_logical_codebase_registration),
        )
        .route(
            "/api/projects/{project_id}/logical-codebase/registrations/{batch_id}",
            get(handlers::get_logical_codebase_registration_batch),
        )
        .route(
            "/api/projects/{project_id}/logical-codebase/registrations/{batch_id}/resume",
            post(handlers::resume_logical_codebase_registration),
        )
        .route(
            "/api/projects/{project_id}/logical-codebase/registrations/{batch_id}/cancel",
            post(handlers::cancel_logical_codebase_registration),
        )
        .route(
            "/api/projects/{project_id}/logical-codebase/pointer-publications",
            get(handlers::list_pointer_publications).post(handlers::create_pointer_publication),
        )
        .route(
            "/api/projects/{project_id}/logical-codebase/pointer-publications/{publication_id}",
            get(handlers::get_pointer_publication),
        )
        .route(
            "/api/projects/{project_id}/logical-codebase/pointer-publications/{publication_id}/retry-repo",
            post(handlers::retry_pointer_publication_repo),
        )
        .route(
            "/api/projects/{project_id}/logical-codebase/pointer-publications/{publication_id}/revoke",
            post(handlers::revoke_pointer_publication),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/pointer-publications",
            get(handlers::list_lc_pointer_publications)
                .post(handlers::create_lc_pointer_publication),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/pointer-publications/{publication_id}",
            get(handlers::get_lc_pointer_publication),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/pointer-publications/{publication_id}/retry-repo",
            post(handlers::retry_lc_pointer_publication_repo),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/pointer-publications/{publication_id}/revoke",
            post(handlers::revoke_lc_pointer_publication),
        )
        .route(
            "/api/projects/{project_id}/issues",
            get(handlers::list_product_issues).post(handlers::create_product_issue),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}",
            delete(handlers::delete_product_issue),
        )
        .route(
            "/api/issues/{issue_id}/lifecycle",
            get(handlers::issue_lifecycle),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/story-specs:generate",
            post(handlers::generate_story_specs),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/story-specs/{story_spec_id}",
            delete(handlers::delete_story_spec),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/design-specs:generate",
            post(handlers::generate_design_specs),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/design-specs/{design_spec_id}",
            delete(handlers::delete_design_spec),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/work-item-plans:prepare",
            post(handlers::prepare_work_item_plan),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/automation-enrollment",
            get(handlers::get_automation_enrollment).put(handlers::put_automation_enrollment),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/automation-enrollment/binding",
            post(handlers::post_automation_enrollment_binding),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/automation-enrollment/rebind",
            post(handlers::post_automation_enrollment_rebind),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/automation-enrollment/lease/takeover",
            post(handlers::post_automation_enrollment_lease_takeover),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/automation-enrollment/continue-plan",
            post(handlers::post_automation_continue_plan),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/automation-enrollment/terminate",
            post(handlers::post_automation_enrollment_terminate),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/automation-enrollment/continue-chain",
            post(handlers::post_automation_enrollment_continue_chain),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/automation-enrollment/control",
            get(handlers::get_automation_enrollment_control),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/automation-target",
            get(handlers::get_automation_target),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/work-item-plans/{plan_id}",
            delete(handlers::delete_work_item_plan),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/work-items/{work_item_id}",
            delete(handlers::delete_work_item),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/work-item-plans/{plan_id}/advance/retry-initialization",
            post(crate::web::advance_plan::post_work_item_plan_advance_retry_initialization),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/work-item-plans/{plan_id}/coding-attempts",
            post(handlers::create_group_coding_attempt),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/work-item-plans/{plan_id}/coding-start",
            get(handlers::get_plan_coding_progress).post(handlers::post_plan_coding_start),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/work-item-plans/{plan_id}/coding-final-confirm",
            post(handlers::post_plan_coding_final_confirm),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/work-item-plans/{plan_id}/coding-workspaces",
            delete(handlers::delete_plan_coding_workspaces),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/work-items/{work_item_id}/coding-attempts",
            post(handlers::create_coding_attempt),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}",
            get(handlers::get_coding_attempt).delete(handlers::delete_coding_attempt),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/diff",
            get(handlers::coding_attempt_diff),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/abort",
            post(handlers::abort_coding_attempt),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/restart",
            post(handlers::restart_coding_attempt),
        )
        .route(
            // C2 Task 12（REQ-CRO-06）：驾驶舱 gate response REST（与 WS
            // GateResponse 同一应用服务）。
            "/api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/gate-responses",
            post(handlers::post_coding_gate_response),
        )
        .route(
            // C2 oracle C-2a/b/c：验证处理转入/决定、triage 记录 GET、并列
            // 证据派生、重跑原计划命令、受限政策读取/重新授权——全部复用
            // 既有引擎服务与 Task 2 命令账本（REST 与 WS/页面同一应用服务）。
            "/api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/verification-triage",
            get(handlers::get_verification_triage_records)
                .post(handlers::post_verification_triage_enter),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/verification-triage/{triage_id}/decision",
            post(handlers::post_verification_triage_decision),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/verification-command-evidence",
            get(handlers::get_verification_command_evidence),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/rerun-planned-command",
            post(handlers::post_rerun_planned_command),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/policy-text",
            get(handlers::get_coding_policy_text),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/policy-reauthorization",
            post(handlers::post_coding_policy_reauthorization),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/choices/{choice_id}/response",
            post(handlers::post_coding_choice_response),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/choices/{choice_id}/responses/{command_id}",
            get(handlers::get_coding_choice_response_status),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/execution-plan/confirm",
            post(handlers::confirm_work_item_execution_plan),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/execution-plan/change-request",
            post(handlers::request_work_item_execution_plan_change),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/artifacts/{artifact_id}",
            get(handlers::coding_attempt_artifact_content),
        )
        .route(
            "/api/coding-attempts/{attempt_id}",
            get(handlers::get_coding_attempt).delete(handlers::delete_coding_attempt),
        )
        .route(
            "/api/coding-attempts/{attempt_id}/diff",
            get(handlers::coding_attempt_diff),
        )
        .route(
            "/api/coding-attempts/{attempt_id}/abort",
            post(handlers::abort_coding_attempt),
        )
        .route(
            "/api/coding-attempts/{attempt_id}/execution-plan/confirm",
            post(handlers::confirm_work_item_execution_plan),
        )
        .route(
            "/api/coding-attempts/{attempt_id}/execution-plan/change-request",
            post(handlers::request_work_item_execution_plan_change),
        )
        .route(
            "/api/coding-attempts/{attempt_id}/artifacts/{artifact_id}",
            get(handlers::coding_attempt_artifact_content),
        )
        .route(
            "/api/workspace-sessions/{session_id}/message",
            post(handlers::workspace_session_message),
        )
        .route(
            "/api/workspace-sessions/{session_id}/run-next",
            post(handlers::workspace_session_run_next),
        )
        .route(
            "/api/workspace-sessions/{session_id}/confirm",
            post(handlers::workspace_session_confirm),
        )
        .route(
            "/api/workspace-sessions/{session_id}/takeover",
            post(handlers::workspace_session_takeover),
        )
        .route(
            "/api/workspace-sessions/{session_id}/choices/{choice_id}/response",
            post(handlers::post_workspace_choice_response),
        )
        .route(
            "/api/workspace-sessions/{session_id}/choices/{choice_id}/responses/{command_id}",
            get(handlers::get_workspace_choice_response_status),
        )
        .route(
            "/api/workspace-sessions/{session_id}/human-actions",
            post(handlers::post_workspace_human_action),
        )
        .route(
            "/api/workspace-sessions/{session_id}/failed-sc-runs/{failed_node_id}/retry",
            post(handlers::post_workspace_failed_sc_run_retry),
        )
        .route(
            "/api/workspace-sessions/{session_id}/lease-diagnostics",
            get(handlers::workspace_session_lease_diagnostics),
        )
        .route(
            "/api/workspace-sessions/{session_id}/timeline-node-details/{node_id}",
            get(handlers::workspace_session_timeline_node_detail),
        )
        .route(
            "/api/workspace-sessions/{session_id}/timeline-node-details/{node_id}/prompt",
            get(handlers::workspace_session_timeline_node_prompt),
        )
        .route(
            "/api/workspace-sessions/{session_id}/timeline-node-details/{node_id}/events/{event_id}/output",
            get(handlers::workspace_session_timeline_event_output),
        )
        .route(
            "/api/workspace-sessions/{session_id}/artifact-versions/{version}",
            get(handlers::workspace_session_artifact_version),
        )
        .route(
            "/api/issues",
            get(handlers::list_issues).post(handlers::create_issue),
        )
        .route("/api/issues/{issue_id}", delete(handlers::delete_issue))
        .route(
            "/api/issues/{issue_id}/rollback/preview",
            post(handlers::issue_rollback_preview),
        )
        .route(
            "/api/issues/{issue_id}/rollback",
            post(handlers::issue_rollback),
        )
        .route(
            "/api/issues/{issue_id}/provider-inputs/{input_ref}",
            get(handlers::provider_input_content),
        )
        .route(
            "/api/issues/{issue_id}/gates/{gate_id}/confirm",
            post(handlers::confirm_gate),
        )
        .route(
            "/api/issues/{issue_id}/gates/{gate_id}/request-change",
            post(handlers::request_gate_change),
        )
        .route(
            "/api/issues/{issue_id}/gates/{gate_id}/terminate",
            post(handlers::terminate_gate),
        )
        .route("/api/tasks/{task_id}/advance", post(handlers::advance_task))
        .route("/api/tasks/{task_id}/confirm", post(handlers::confirm_task))
        .route("/api/tasks/{task_id}/stop", post(handlers::stop_task))
        .route(
            "/api/tasks/{task_id}/rollback/preview",
            post(handlers::rollback_preview),
        )
        .route(
            "/api/tasks/{task_id}/rollback",
            post(handlers::rollback_task),
        )
        .route(
            "/api/artifacts/{artifact_ref}",
            get(handlers::artifact_content),
        )
        .route("/api/files/content", get(handlers::file_content))
        .route("/api/files/diff", get(handlers::file_diff))
        .route(
            "/api/workspace-sessions/{session_id}/ws",
            get(workspace_ws_handler::workspace_ws),
        )
        .route(
            "/api/ws/workspace/{session_id}",
            get(workspace_ws_handler::workspace_ws),
        )
        .route(
            "/ws/coding-attempts/{attempt_id}",
            get(coding_ws_handler::coding_ws),
        )
        .route(
            "/ws/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}",
            get(coding_ws_handler::scoped_coding_ws),
        );

    // C-4 T7：证据查询路由仅在回环监听（evidence_enabled）时挂载。
    let router = if evidence_enabled {
        router.route("/api/evidence-query", post(handlers::evidence_query))
    } else {
        router
    };

    let router = if test_controls::test_controls_enabled() {
        router
            .route(
                "/api/test/workspace-sessions/{session_id}/ws/drop",
                post(test_controls::drop_workspace_socket),
            )
            .route(
                "/api/test/workspace-sessions/{session_id}/ws/reject-next",
                post(test_controls::reject_next_workspace_sockets),
            )
            .route(
                "/api/test/workspace-sessions/{session_id}/permission-fixture",
                post(test_controls::enable_permission_fixture),
            )
            .route(
                "/api/test/workspace-sessions/{session_id}/review-fixture",
                post(test_controls::enable_review_fixture),
            )
            .route(
                "/api/test/coding-attempts/{attempt_id}/review-fixture",
                post(test_controls::enable_review_fixture),
            )
            .route(
                "/api/test/workspace-sessions/large-fixture",
                post(test_controls::seed_large_workspace_fixture),
            )
            .route(
                "/api/test/coding-attempts/role-run-fixture",
                post(test_controls::seed_coding_role_run_fixture),
            )
            .route(
                "/api/test/permission-timeout",
                post(test_controls::set_permission_timeout),
            )
            .route("/api/test/ws-timeout", post(test_controls::set_ws_timeout))
    } else {
        router
    };

    // 批2 T1：认证/来源校验中间件挂在最外层——晚于全部 route 注册、先于任何
    // handler/WS upgrade 执行。`api_auth` 未装配（进程内测试默认）时直通。
    router
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::web::api_auth::enforce_api_auth,
        ))
        .with_state(state)
}

/// launcher 依赖的就绪行前缀契约。修改即破坏 launcher 解析，须同步更新 bin/aria.js 与回归测试。
pub const LISTENING_LINE_PREFIX: &str = "aria web listening on http://";

/// 生成就绪行（统一格式来源）。
pub fn listening_line(addr: &SocketAddr) -> String {
    format!("{LISTENING_LINE_PREFIX}{addr}")
}

/// 证据端点发现文件的相对路径（写于 workspace 根，T7）。
pub const WEB_ENDPOINT_FILE: &str = ".aria/web-endpoint";

/// 回环 host 白名单（设计 §3.3）：证据路由仅在这些 host 上挂载。
pub fn is_loopback_host(host: &str) -> bool {
    // M4（fix round 1）：去括号 + trim 后再匹配，识别 `[::1]` 等括号形式。
    let host = host.trim();
    let host = host
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(host);
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

/// 把绑定成功的端口原子写入 `.aria/web-endpoint`（内容为纯端口号）。
pub fn write_web_endpoint_file(workspace_root: &Path, port: u16) -> std::io::Result<()> {
    let path = web_endpoint_path(workspace_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp_path = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&temp_path, port.to_string())?;
    std::fs::rename(&temp_path, &path)?;
    Ok(())
}

fn web_endpoint_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join(WEB_ENDPOINT_FILE)
}

/// M3（fix round 1）：证据路由未挂载（非回环 host）时跳过端口文件写入，
/// 避免端口文件留在 workspace 根却无可用的证据路由。
fn maybe_write_web_endpoint_file(evidence_enabled: bool, workspace_root: &Path, port: u16) {
    if !evidence_enabled {
        return;
    }
    if let Err(error) = write_web_endpoint_file(workspace_root, port) {
        eprintln!(
            "warning: write {}: {error}",
            web_endpoint_path(workspace_root).display()
        );
    }
}

pub async fn serve_web(
    workspace_root: std::path::PathBuf,
    host: String,
    port: Option<u16>,
    work_item_plan_single_candidate: bool,
) -> anyhow::Result<()> {
    let product_paths = ProductAppPaths::new(workspace_root.join(".aria"));
    ensure_product_data_schema(&product_paths)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    let addr: SocketAddr = format!("{}:{}", host, port.unwrap_or(0)).parse()?;
    let events = EventHub::new();
    let state = web_app_state(
        workspace_root.clone(),
        crate::web::runtime::WebRuntime::new_real_with_events(
            workspace_root.clone(),
            events.clone(),
        )
        .map_err(|error| anyhow::anyhow!("{:?}: {}", error.code, error.message))?,
        events,
        work_item_plan_single_candidate,
    );
    refresh_provider_health_for_startup(&state).await;
    // P2 Task 6（§3.2）：无 attach 启动扫描——恢复已认领/在途 coding run，
    // 先于 autopilot 有界 tick 执行一次；失败 fail-visible，不阻塞服务。
    if let Err(error) =
        crate::web::autopilot_orchestrator::reconcile_claimed_coding_runs_once(&state).await
    {
        eprintln!("[aria-startup] coding run startup reconcile failed: {error}");
    }
    let static_service = crate::web::static_assets::static_dist_service();
    let evidence_enabled = is_loopback_host(&host);

    // 批2 T1：先 bind 拿真实端口，再装配控制面认证（默认受信 Origin/Host 精确
    // 来自该监听地址；bind 非 loopback 时必须显式配置 ARIA_API_TRUSTED_ORIGINS）。
    let listener = TcpListener::bind(addr).await?;
    let bound_addr = listener.local_addr()?;
    let explicit_origins = crate::web::api_auth::parse_trusted_origins_env(
        &std::env::var(crate::web::api_auth::TRUSTED_ORIGINS_ENV).unwrap_or_default(),
    );
    let auth = std::sync::Arc::new(
        crate::web::api_auth::ApiAuthState::for_bind(&bound_addr, &explicit_origins)
            .map_err(|error| anyhow::anyhow!("api auth bootstrap failed: {error}"))?,
    );
    eprintln!(
        "{}",
        crate::web::api_auth::delivery_notice(&auth, owner_terminal_attached())
    );
    let state = state.with_api_auth(auth);
    let app = build_web_router_with_evidence(state.clone(), evidence_enabled).fallback(
        move |req: axum::extract::Request| {
            let static_service = static_service.clone();
            async move { crate::web::static_assets::serve_static(static_service, req).await }
        },
    );
    // P1 WIGA Task 6：随服务器生命周期启动 autopilot 有界扫描（事件仅唤醒）。
    let wake_rx = orchestrator_wake_subscribe(&state);
    let orchestrator = std::sync::Arc::new(
        crate::web::autopilot_orchestrator::AutopilotOrchestrator::new(
            state,
            crate::web::autopilot_orchestrator::OrchestratorConfig::default(),
        ),
    );
    let orchestrator_task = tokio::spawn(async move { orchestrator.run(wake_rx).await });

    maybe_write_web_endpoint_file(evidence_enabled, &workspace_root, bound_addr.port());
    eprintln!("{}", listening_line(&bound_addr));
    axum::serve(listener, app).await?;
    orchestrator_task.abort();
    Ok(())
}

/// 批2 T1：owner 受信终端判定（stderr 是否 TTY）。无 TTY 时 bootstrap token
/// 不交付（显式报告缺少交付前提），绝不绕道集中日志。
fn owner_terminal_attached() -> bool {
    unsafe { libc::isatty(libc::STDERR_FILENO) == 1 }
}

fn orchestrator_wake_subscribe(state: &WebAppState) -> tokio::sync::watch::Receiver<bool> {
    state.autopilot_wake.subscribe()
}

fn web_app_state(
    workspace_root: PathBuf,
    runtime: crate::web::runtime::WebRuntime,
    events: EventHub,
    work_item_plan_single_candidate: bool,
) -> WebAppState {
    WebAppState::with_events(workspace_root, runtime, events)
        .with_work_item_plan_single_candidate(work_item_plan_single_candidate)
}

async fn refresh_provider_health_for_startup(state: &WebAppState) {
    if !state.test_provider_enabled {
        state.refresh_provider_health().await;
    }
    let snapshot = state.provider_health.latest_diagnostic();
    crate::web::provider_probe::emit_provider_probe_notice(
        snapshot.as_ref(),
        state.provider_health.degraded(),
    );
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use chrono::Utc;
    use tempfile::tempdir;
    use tower::ServiceExt;

    use super::{
        WEB_ENDPOINT_FILE, build_web_router, build_web_router_with_evidence, is_loopback_host,
        maybe_write_web_endpoint_file, refresh_provider_health_for_startup,
        write_web_endpoint_file,
    };
    use crate::cross_cutting::aria_state_paths::AriaStatePaths;
    use crate::cross_cutting::bounded_command_runner::{
        BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
    };
    use crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
    use crate::cross_cutting::provider_health::{ProviderHealthClock, ProviderHealthService};
    use crate::web::events::EventHub;
    use crate::web::runtime::WebRuntime;
    use crate::web::state::WebAppState;

    struct FixedClock;

    impl ProviderHealthClock for FixedClock {
        fn now(&self) -> chrono::DateTime<Utc> {
            Utc::now()
        }
    }

    struct MissingRunner;

    #[async_trait::async_trait]
    impl BoundedCommandRunner for MissingRunner {
        async fn run(
            &self,
            request: BoundedCommandRequest,
        ) -> Result<BoundedCommandResult, BoundedCommandError> {
            Err(BoundedCommandError::CommandMissing {
                executable: request.executable,
                details: "not found".to_string(),
            })
        }
    }

    fn state(root: &std::path::Path) -> WebAppState {
        let runner: Arc<dyn BoundedCommandRunner> = Arc::new(MissingRunner);
        let health = Arc::new(ProviderHealthService::with_dependencies(
            AriaStatePaths::from_workspace_root(root),
            runner.clone(),
            Arc::new(FixedClock),
            Duration::from_secs(1),
            4096,
        ));
        let gate = Arc::new(ProviderAvailabilityGate::new(health.clone()));
        let mut state =
            WebAppState::new(root.to_path_buf(), WebRuntime::new_fake(root.to_path_buf()))
                .with_provider_health(health, gate, runner);
        state.test_provider_enabled = false;
        state
    }

    #[test]
    fn web_app_state_applies_single_candidate_rollout_only_when_requested_at_startup() {
        let root = tempdir().expect("root");
        let enabled = super::web_app_state(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
            EventHub::new(),
            true,
        );
        let disabled = super::web_app_state(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
            EventHub::new(),
            false,
        );

        assert!(enabled.work_item_plan_single_candidate());
        assert!(!disabled.work_item_plan_single_candidate());
    }

    #[tokio::test]
    async fn providers_status_routes_exist_and_health_stays_ok_when_degraded() {
        let root = tempdir().expect("root");
        let state = state(root.path());
        let app = build_web_router(state);

        for (method, uri) in [
            ("GET", "/api/health"),
            ("GET", "/api/providers/status"),
            ("POST", "/api/providers/recheck"),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::OK, "{method} {uri}");
        }
    }

    #[tokio::test]
    async fn providers_status_startup_refresh_storage_error_does_not_block_router() {
        let root = tempdir().expect("root");
        let blocked_root = root.path().join("not-a-directory");
        std::fs::write(&blocked_root, "blocked").expect("blocked root");
        let mut state = state(&blocked_root);
        state.test_provider_enabled = false;

        refresh_provider_health_for_startup(&state).await;
        let response = build_web_router(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/api/providers/status")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert!(state.provider_health.degraded());
        assert_eq!(state.provider_health.latest_diagnostic().generation, 1);
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn providers_status_startup_fake_mode_skips_real_refresh() {
        let root = tempdir().expect("root");
        let mut state = state(root.path());
        state.test_provider_enabled = true;

        refresh_provider_health_for_startup(&state).await;

        assert_eq!(state.provider_health.latest_diagnostic().generation, 0);
    }

    #[test]
    fn web_evidence_endpoint_file_writes_plain_port_number() {
        let root = tempdir().expect("root");
        write_web_endpoint_file(root.path(), 43_210).expect("write endpoint file");

        let content = std::fs::read_to_string(root.path().join(WEB_ENDPOINT_FILE))
            .expect("read endpoint file");
        assert_eq!(
            content, "43210",
            "endpoint file must hold the plain port number"
        );
    }

    #[test]
    fn is_loopback_host_accepts_only_loopback_bindings() {
        for host in ["127.0.0.1", "localhost", "::1", "[::1]", " [::1] "] {
            assert!(is_loopback_host(host), "{host} must be loopback");
        }
        for host in ["0.0.0.0", "::", "192.168.1.10", "example.com"] {
            assert!(!is_loopback_host(host), "{host} must not be loopback");
        }
    }

    #[test]
    fn web_endpoint_file_skipped_when_evidence_disabled() {
        let root = tempdir().expect("root");
        maybe_write_web_endpoint_file(false, root.path(), 43_210);
        assert!(
            !root.path().join(WEB_ENDPOINT_FILE).exists(),
            "non-loopback (evidence disabled) must not write endpoint file"
        );
    }

    #[test]
    fn web_endpoint_file_written_when_evidence_enabled() {
        let root = tempdir().expect("root");
        maybe_write_web_endpoint_file(true, root.path(), 43_210);
        assert_eq!(
            std::fs::read_to_string(root.path().join(WEB_ENDPOINT_FILE))
                .expect("read endpoint file"),
            "43210"
        );
    }

    #[tokio::test]
    async fn web_evidence_route_absent_when_loopback_disabled() {
        let root = tempdir().expect("root");
        let app = build_web_router_with_evidence(state(root.path()), false);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/evidence-query")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "evidence route must not be mounted when host is not loopback"
        );
    }

    #[tokio::test]
    async fn web_evidence_route_present_when_loopback_enabled() {
        let root = tempdir().expect("root");
        let app = build_web_router_with_evidence(state(root.path()), true);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/evidence-query")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_ne!(
            response.status(),
            StatusCode::NOT_FOUND,
            "evidence route must be mounted when host is loopback"
        );
    }

    /// 批2 Task 1：控制面认证真实 router 测试（表驱动 401/403、一次性交换、
    /// Host/Origin 前置、零副作用、真实 WS 握手、材料不泄漏、未装配直通）。
    mod api_auth_gates {
        use axum::body::Body;
        use axum::http::{HeaderValue, Request, StatusCode};
        use serde_json::json;
        use tempfile::tempdir;
        use tower::ServiceExt;

        use super::super::build_web_router;
        use super::state;
        use crate::web::api_auth::ApiAuthState;
        use crate::web::state::WebAppState;

        const AUTH_PORT: u16 = 39721;

        fn trusted_host() -> &'static str {
            "127.0.0.1:39721"
        }

        fn trusted_origin() -> &'static str {
            "http://127.0.0.1:39721"
        }

        fn armed_state(root: &std::path::Path) -> (WebAppState, String) {
            let bind: std::net::SocketAddr =
                format!("127.0.0.1:{AUTH_PORT}").parse().expect("bind addr");
            let auth = std::sync::Arc::new(ApiAuthState::for_bind(&bind, &[]).expect("auth"));
            let token = auth.bootstrap_token_hex();
            (state(root).with_api_auth(auth), token)
        }

        fn valid_task_body() -> Body {
            Body::from(
                json!({
                    "request_text": "auth probe",
                    "change_id": "change_auth_0001",
                    "policy_preset": "balanced",
                    "provider_mode": "fake",
                    "timeout_secs": 60,
                })
                .to_string(),
            )
        }

        fn ws_headers(builder: axum::http::request::Builder) -> axum::http::request::Builder {
            builder
                .header("connection", "upgrade")
                .header("upgrade", "websocket")
                .header("sec-websocket-version", "13")
                .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        }

        async fn body_text(response: axum::response::Response) -> String {
            let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
                .await
                .expect("body bytes");
            String::from_utf8(bytes.to_vec()).expect("utf-8 body")
        }

        async fn exchange(app: &axum::Router, token: &str) -> axum::response::Response {
            exchange_with(app, token, trusted_host(), trusted_origin()).await
        }

        async fn exchange_with(
            app: &axum::Router,
            token: &str,
            host: &str,
            origin: &str,
        ) -> axum::response::Response {
            app.clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/auth/session")
                        .header("host", host)
                        .header("origin", origin)
                        .header("content-type", "application/json")
                        .body(Body::from(json!({ "token": token }).to_string()))
                        .expect("exchange request"),
                )
                .await
                .expect("exchange response")
        }

        async fn bootstrap_session(app: &axum::Router, token: &str) -> (String, String) {
            let response = exchange(app, token).await;
            assert_eq!(
                response.status(),
                StatusCode::NO_CONTENT,
                "首次交换必须成功：{}",
                body_text(response).await
            );
            let response = exchange(app, token);
            let _ = response;
            unreachable!("上一次调用已消费 response，这里不会执行")
        }

        /// T1-RED 表驱动：health 公开；其余 mutation/敏感 GET/三类 WS（含 alias）
        /// 在受信 Host+Origin 下缺会话 → 401；localhost 无豁免。
        #[tokio::test]
        async fn api_auth_gates_every_non_health_route_without_session() {
            let root = tempdir().expect("root");
            let (st, _token) = armed_state(root.path());
            let app = build_web_router(st);

            let health = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/api/health")
                        .body(Body::empty())
                        .expect("health request"),
                )
                .await
                .expect("health response");
            assert_eq!(health.status(), StatusCode::OK, "GET /api/health 公开");

            struct Case {
                method: &'static str,
                uri: &'static str,
                ws: bool,
                body: Option<Body>,
            }
            let cases = [
                Case {
                    method: "GET",
                    uri: "/api/providers/status",
                    ws: false,
                    body: None,
                },
                Case {
                    method: "GET",
                    uri: "/api/runtime-info",
                    ws: false,
                    body: None,
                },
                Case {
                    method: "GET",
                    uri: "/api/tasks",
                    ws: false,
                    body: None,
                },
                Case {
                    method: "GET",
                    uri: "/api/workspaces",
                    ws: false,
                    body: None,
                },
                Case {
                    method: "POST",
                    uri: "/api/tasks",
                    ws: false,
                    body: Some(valid_task_body()),
                },
                Case {
                    method: "POST",
                    uri: "/api/providers/recheck",
                    ws: false,
                    body: None,
                },
                Case {
                    method: "DELETE",
                    uri: "/api/workspaces/ws_0001",
                    ws: false,
                    body: None,
                },
                Case {
                    method: "GET",
                    uri: "/api/workspace-sessions/sess_1/ws",
                    ws: true,
                    body: None,
                },
                Case {
                    method: "GET",
                    uri: "/api/ws/workspace/sess_1",
                    ws: true,
                    body: None,
                },
                Case {
                    method: "GET",
                    uri: "/ws/coding-attempts/attempt_1",
                    ws: true,
                    body: None,
                },
                Case {
                    method: "GET",
                    uri: "/ws/projects/project_1/issues/issue_1/coding-attempts/attempt_1",
                    ws: true,
                    body: None,
                },
                Case {
                    method: "GET",
                    uri: "/api/image-create/sessions/img_1/chat",
                    ws: true,
                    body: None,
                },
            ];
            for case in cases {
                let mut builder = Request::builder()
                    .method(case.method)
                    .uri(case.uri)
                    .header("host", trusted_host())
                    .header("origin", trusted_origin());
                if case.ws {
                    builder = ws_headers(builder);
                }
                let response = app
                    .clone()
                    .oneshot(
                        builder
                            .body(case.body.unwrap_or(Body::empty()))
                            .expect("request"),
                    )
                    .await
                    .expect("response");
                assert_eq!(
                    response.status(),
                    StatusCode::UNAUTHORIZED,
                    "{} {} 缺会话必须 401（localhost 不豁免）",
                    case.method,
                    case.uri
                );
            }
        }

        /// T1-RED 一次性交换：仅 body 接 token、原子消费、Strict cookie、
        /// 会话检查 401/204、复用/错误 token → 401、交换本身需受信 Origin。
        #[tokio::test]
        async fn api_auth_exchange_consumes_token_once_and_sets_strict_cookie() {
            let root = tempdir().expect("root");
            let (st, token) = armed_state(root.path());
            let app = build_web_router(st);

            // 仅 body 接 token：query 携带 token + 空 body 不构成交换。
            let query_attempt = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(format!("/api/auth/session?token={token}"))
                        .header("host", trusted_host())
                        .header("origin", trusted_origin())
                        .header("content-type", "application/json")
                        .body(Body::empty())
                        .expect("query attempt"),
                )
                .await
                .expect("query attempt response");
            assert_ne!(
                query_attempt.status(),
                StatusCode::NO_CONTENT,
                "query 中的 token 不得被消费"
            );
            assert!(
                query_attempt.headers().get("set-cookie").is_none(),
                "失败交换不得签发 cookie"
            );

            // 合法首次交换 → 204 + Strict cookie。
            let response = exchange(&app, &token).await;
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
            let cookie_line = response
                .headers()
                .get("set-cookie")
                .and_then(|value| value.to_str().ok())
                .expect("set-cookie")
                .to_string();
            assert!(
                cookie_line.contains("HttpOnly"),
                "cookie 须 HttpOnly：{cookie_line}"
            );
            assert!(
                cookie_line.contains("SameSite=Strict"),
                "cookie 须 SameSite=Strict"
            );
            assert!(cookie_line.contains("Path=/"), "cookie 须 Path=/");
            assert!(
                !cookie_line.contains("Secure"),
                "本地 http 受信来源不加 Secure"
            );
            let session = cookie_line
                .split(';')
                .next()
                .expect("cookie pair")
                .trim()
                .strip_prefix("aria_session=")
                .expect("aria_session")
                .to_string();
            assert_eq!(session.len(), 64, "会话 ID 为 32 字节 hex");
            assert_ne!(session, token, "会话 ID 与 bootstrap token 不同值");

            // 会话检查：已认证 204 空体；未认证 401；响应不回显 token。
            let authed_check = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/api/auth/session")
                        .header("host", trusted_host())
                        .header("cookie", format!("aria_session={session}"))
                        .body(Body::empty())
                        .expect("check"),
                )
                .await
                .expect("check response");
            assert_eq!(authed_check.status(), StatusCode::NO_CONTENT);
            assert!(
                body_text(authed_check).await.is_empty(),
                "204 不回显任何材料"
            );

            let anon_check = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/api/auth/session")
                        .header("host", trusted_host())
                        .body(Body::empty())
                        .expect("anon check"),
                )
                .await
                .expect("anon check response");
            assert_eq!(anon_check.status(), StatusCode::UNAUTHORIZED);
            assert!(!body_text(anon_check).await.contains(&token));

            // 复用 bootstrap / 错误 token → 401 且不签发 cookie。
            for bad_token in [&token, "deadbeef", "not-hex"] {
                let response = exchange(&app, bad_token).await;
                assert_eq!(
                    response.status(),
                    StatusCode::UNAUTHORIZED,
                    "token={bad_token} 须统一 401"
                );
                assert!(response.headers().get("set-cookie").is_none());
                let body = body_text(response).await;
                assert!(!body.contains(&token), "错误响应不得回显 token");
                assert!(!body.contains("deadbeef") || bad_token != "deadbeef");
            }

            // 交换是状态变更：缺 Origin → 403（先于 handler）。
            let no_origin = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/auth/session")
                        .header("host", trusted_host())
                        .header("content-type", "application/json")
                        .body(Body::from(json!({ "token": token }).to_string()))
                        .expect("no origin exchange"),
                )
                .await
                .expect("no origin response");
            assert_eq!(
                no_origin.status(),
                StatusCode::FORBIDDEN,
                "mutation 缺 Origin 在 handler 前拒绝"
            );
        }

        /// T1-RED Host/Origin 前置（已认证会话）：mutation/WS 必须受信精确 Origin，
        /// 敏感 GET 无 Origin 仍认证；非法/null Origin、非法 Host → 403。
        #[tokio::test]
        async fn api_auth_enforces_host_and_origin_before_handlers() {
            let root = tempdir().expect("root");
            let (st, token) = armed_state(root.path());
            let app = build_web_router(st);
            let response = exchange(&app, &token).await;
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
            let cookie_line = response
                .headers()
                .get("set-cookie")
                .and_then(|value| value.to_str().ok())
                .expect("set-cookie")
                .to_string();
            let session = cookie_line
                .split(';')
                .next()
                .expect("pair")
                .trim()
                .strip_prefix("aria_session=")
                .expect("session")
                .to_string();
            let cookie = format!("aria_session={session}");

            let request =
                |origin: Option<&str>, host: Option<&str>, uri: &'static str, ws: bool| {
                    let mut builder = Request::builder().uri(uri);
                    if let Some(origin) = origin {
                        builder = builder.header("origin", origin);
                    }
                    if let Some(host) = host {
                        builder = builder.header("host", host);
                    } else {
                        builder = builder.header("host", trusted_host());
                    }
                    builder = builder.header("cookie", cookie.as_str());
                    if ws {
                        builder = ws_headers(builder);
                    }
                    builder.body(Body::empty()).expect("request")
                };

            // 敏感 GET：无 Origin 仍认证 + Host 正确 → 200。
            let response = app
                .clone()
                .oneshot(request(None, None, "/api/tasks", false))
                .await
                .expect("no-origin GET");
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "敏感 GET 无 Origin 仍放行"
            );

            // 受信 Origin GET → 200。
            let response = app
                .clone()
                .oneshot(request(Some(trusted_origin()), None, "/api/tasks", false))
                .await
                .expect("trusted GET");
            assert_eq!(response.status(), StatusCode::OK);

            // null / 恶意 Origin → 403。
            for evil in ["null", "http://evil.example", "https://127.0.0.1:39721"] {
                let response = app
                    .clone()
                    .oneshot(request(Some(evil), None, "/api/tasks", false))
                    .await
                    .expect("evil GET");
                assert_eq!(
                    response.status(),
                    StatusCode::FORBIDDEN,
                    "Origin={evil} 须 403"
                );
            }

            // 非法 Host（Origin 受信）→ 403。
            for bad_host in ["evil.example:4317", "127.0.0.1:39722", "localhost"] {
                let response = app
                    .clone()
                    .oneshot(request(
                        Some(trusted_origin()),
                        Some(bad_host),
                        "/api/tasks",
                        false,
                    ))
                    .await
                    .expect("bad host GET");
                assert_eq!(
                    response.status(),
                    StatusCode::FORBIDDEN,
                    "Host={bad_host} 须 403"
                );
            }

            // loopback Host 变体（带受信端口）→ 200。
            let response = app
                .clone()
                .oneshot(request(None, Some("localhost:39721"), "/api/tasks", false))
                .await
                .expect("loopback variant GET");
            assert_eq!(response.status(), StatusCode::OK);

            // mutation：缺 Origin → 403；恶意 Origin → 403；受信 → 通过认证门。
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/tasks")
                        .header("host", trusted_host())
                        .header("cookie", cookie.as_str())
                        .header("content-type", "application/json")
                        .body(valid_task_body())
                        .expect("mutation no origin"),
                )
                .await
                .expect("mutation response");
            assert_eq!(
                response.status(),
                StatusCode::FORBIDDEN,
                "mutation 缺 Origin 须 403"
            );

            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/tasks")
                        .header("host", trusted_host())
                        .header("origin", "http://evil.example")
                        .header("cookie", cookie.as_str())
                        .header("content-type", "application/json")
                        .body(valid_task_body())
                        .expect("mutation evil origin"),
                )
                .await
                .expect("mutation response");
            assert_eq!(response.status(), StatusCode::FORBIDDEN);

            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/tasks")
                        .header("host", trusted_host())
                        .header("origin", trusted_origin())
                        .header("cookie", cookie.as_str())
                        .header("content-type", "application/json")
                        .body(valid_task_body())
                        .expect("mutation trusted"),
                )
                .await
                .expect("mutation response");
            assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
            assert_ne!(response.status(), StatusCode::FORBIDDEN);

            // WS：无 Origin / 恶意 Origin → 403；受信 Origin+cookie → 非 401/403。
            let response = app
                .clone()
                .oneshot(request(None, None, "/api/workspace-sessions/s1/ws", true))
                .await
                .expect("ws no origin");
            assert_eq!(
                response.status(),
                StatusCode::FORBIDDEN,
                "WS 缺 Origin 须 403"
            );

            let response = app
                .clone()
                .oneshot(request(
                    Some("null"),
                    None,
                    "/api/workspace-sessions/s1/ws",
                    true,
                ))
                .await
                .expect("ws null origin");
            assert_eq!(response.status(), StatusCode::FORBIDDEN);

            let response = app
                .clone()
                .oneshot(request(
                    Some(trusted_origin()),
                    None,
                    "/api/workspace-sessions/s1/ws",
                    true,
                ))
                .await
                .expect("ws trusted");
            assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
            assert_ne!(response.status(), StatusCode::FORBIDDEN);
        }

        /// T1-RED 零副作用：被拒 mutation 不创建对象（副作用/provider 启动为 0）。
        #[tokio::test]
        async fn api_auth_rejection_produces_zero_side_effects() {
            let root = tempdir().expect("root");
            let (st, token) = armed_state(root.path());
            let app = build_web_router(st);

            // 401：缺会话的合法 mutation。
            let rejected = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/tasks")
                        .header("host", trusted_host())
                        .header("origin", trusted_origin())
                        .header("content-type", "application/json")
                        .body(valid_task_body())
                        .expect("unauth mutation"),
                )
                .await
                .expect("response");
            assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);

            // 交换会话后确认任务数为 0。
            let response = exchange(&app, &token).await;
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
            let cookie_line = response
                .headers()
                .get("set-cookie")
                .and_then(|value| value.to_str().ok())
                .expect("set-cookie")
                .to_string();
            let session = cookie_line
                .split(';')
                .next()
                .expect("pair")
                .trim()
                .strip_prefix("aria_session=")
                .expect("session")
                .to_string();
            let cookie = format!("aria_session={session}");

            let list_after_401 = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/api/tasks")
                        .header("host", trusted_host())
                        .header("cookie", cookie.as_str())
                        .body(Body::empty())
                        .expect("list"),
                )
                .await
                .expect("list response");
            assert_eq!(list_after_401.status(), StatusCode::OK);
            let payload: serde_json::Value =
                serde_json::from_str(&body_text(list_after_401).await).expect("json");
            assert_eq!(
                payload["tasks"].as_array().map(Vec::len),
                Some(0),
                "被拒请求不得产生副作用"
            );

            // 403：有会话 + 恶意 Origin 的 mutation 同样零副作用。
            let rejected = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/tasks")
                        .header("host", trusted_host())
                        .header("origin", "http://evil.example")
                        .header("cookie", cookie.as_str())
                        .header("content-type", "application/json")
                        .body(valid_task_body())
                        .expect("evil mutation"),
                )
                .await
                .expect("response");
            assert_eq!(rejected.status(), StatusCode::FORBIDDEN);

            let list_after_403 = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/api/tasks")
                        .header("host", trusted_host())
                        .header("cookie", cookie.as_str())
                        .body(Body::empty())
                        .expect("list"),
                )
                .await
                .expect("list response");
            let payload: serde_json::Value =
                serde_json::from_str(&body_text(list_after_403).await).expect("json");
            assert_eq!(payload["tasks"].as_array().map(Vec::len), Some(0));

            // 对照组：认证 + 受信 Origin 的同请求创建 1 个任务（探针有效）。
            let created = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/tasks")
                        .header("host", trusted_host())
                        .header("origin", trusted_origin())
                        .header("cookie", cookie.as_str())
                        .header("content-type", "application/json")
                        .body(valid_task_body())
                        .expect("allowed mutation"),
                )
                .await
                .expect("created response");
            assert_eq!(created.status(), StatusCode::OK, "对照创建须成功");

            let list_after_create = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/api/tasks")
                        .header("host", trusted_host())
                        .header("cookie", cookie.as_str())
                        .body(Body::empty())
                        .expect("list"),
                )
                .await
                .expect("list response");
            let payload: serde_json::Value =
                serde_json::from_str(&body_text(list_after_create).await).expect("json");
            assert_eq!(payload["tasks"].as_array().map(Vec::len), Some(1));
        }

        /// T1-RED 真实 TCP WS 握手：认证/Origin 在 upgrade 前强制；alias 同样受限。
        #[tokio::test]
        async fn api_auth_ws_handshake_enforced_on_real_server() {
            let root = tempdir().expect("root");
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind");
            let addr = listener.local_addr().expect("addr");
            let auth = std::sync::Arc::new(ApiAuthState::for_bind(&addr, &[]).expect("auth"));
            let token = auth.bootstrap_token_hex();
            let app = build_web_router(state(root.path()).with_api_auth(auth));
            let server_app = app.clone();
            tokio::spawn(async move {
                axum::serve(listener, server_app).await.expect("serve");
            });

            let host = format!("127.0.0.1:{}", addr.port());
            let origin = format!("http://{host}");
            let ws_url = format!("ws://{host}/api/workspace-sessions/sess_ws1/ws");
            let alias_url = format!("ws://{host}/api/ws/workspace/sess_ws1");

            let request_with = |url: &str, cookie: Option<String>, origin: Option<String>| {
                use tokio_tungstenite::tungstenite::client::IntoClientRequest;
                let mut request = url.into_client_request().expect("client request");
                if let Some(cookie) = cookie {
                    request
                        .headers_mut()
                        .insert("cookie", HeaderValue::from_str(&cookie).expect("cookie"));
                }
                if let Some(origin) = origin {
                    request
                        .headers_mut()
                        .insert("origin", HeaderValue::from_str(&origin).expect("origin"));
                }
                request
            };

            // 无 cookie → 握手拒绝（401）。
            let no_cookie = request_with(&ws_url, None, Some(origin.clone()));
            assert!(
                tokio_tungstenite::connect_async(no_cookie).await.is_err(),
                "无会话的 WS upgrade 必须被拒"
            );

            // 与运行中服务器共享同一 auth 状态完成交换。
            let response = exchange_with(&app, &token, &host, &origin).await;
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
            let cookie_line = response
                .headers()
                .get("set-cookie")
                .and_then(|value| value.to_str().ok())
                .expect("set-cookie")
                .to_string();
            let session = cookie_line
                .split(';')
                .next()
                .expect("pair")
                .trim()
                .strip_prefix("aria_session=")
                .expect("session")
                .to_string();
            let cookie = format!("aria_session={session}");

            // 有会话 + 恶意 Origin → 握手拒绝（403）。
            let evil = request_with(
                &ws_url,
                Some(cookie.clone()),
                Some("http://evil.example".to_string()),
            );
            assert!(
                tokio_tungstenite::connect_async(evil).await.is_err(),
                "恶意 Origin 的 WS upgrade 必须被拒"
            );

            // 有会话 + 受信 Origin → 握手成功（101）。
            let allowed = request_with(&ws_url, Some(cookie.clone()), Some(origin.clone()));
            let (stream, response) = tokio_tungstenite::connect_async(allowed)
                .await
                .expect("authenticated handshake must succeed");
            assert_eq!(response.status().as_u16(), 101, "upgrade 必须完成");
            drop(stream);

            // alias 路径同样受认证保护。
            let alias_no_cookie = request_with(&alias_url, None, Some(origin.clone()));
            assert!(
                tokio_tungstenite::connect_async(alias_no_cookie)
                    .await
                    .is_err(),
                "alias WS 无会话必须被拒"
            );
        }

        /// T1-RED 材料不泄漏：token/会话不出现在 env 与任何错误响应体。
        #[tokio::test]
        async fn api_auth_token_absent_from_env_and_response_bodies() {
            let root = tempdir().expect("root");
            let (st, token) = armed_state(root.path());
            let app = build_web_router(st);
            let response = exchange(&app, &token).await;
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
            let cookie_line = response
                .headers()
                .get("set-cookie")
                .and_then(|value| value.to_str().ok())
                .expect("set-cookie")
                .to_string();
            let session = cookie_line
                .split(';')
                .next()
                .expect("pair")
                .trim()
                .strip_prefix("aria_session=")
                .expect("session")
                .to_string();

            let bodies = [
                // 401 未认证响应体
                body_text(
                    app.clone()
                        .oneshot(
                            Request::builder()
                                .uri("/api/tasks")
                                .header("host", trusted_host())
                                .header("origin", trusted_origin())
                                .body(Body::empty())
                                .expect("unauth"),
                        )
                        .await
                        .expect("unauth response"),
                )
                .await,
                // 403 恶意 Origin 响应体（带会话）
                body_text(
                    app.clone()
                        .oneshot(
                            Request::builder()
                                .method("POST")
                                .uri("/api/tasks")
                                .header("host", trusted_host())
                                .header("origin", "http://evil.example")
                                .header("cookie", format!("aria_session={session}"))
                                .header("content-type", "application/json")
                                .body(valid_task_body())
                                .expect("evil"),
                        )
                        .await
                        .expect("evil response"),
                )
                .await,
                // 复用 token 的 401 响应体
                body_text(exchange(&app, &token).await).await,
            ];
            for (index, body) in bodies.iter().enumerate() {
                assert!(!body.contains(&token), "响应 {index} 泄漏 bootstrap token");
                assert!(!body.contains(&session), "响应 {index} 泄漏会话 ID");
            }

            // fixture 环境枚举：env 中无任何入站认证材料。
            for (key, value) in std::env::vars_os() {
                let value = value.to_string_lossy();
                assert!(
                    !value.contains(&token) && !value.contains(&session),
                    "env 变量 {:?} 含认证材料",
                    key.to_string_lossy()
                );
            }
        }

        /// T1-RED 未装配 api_auth 的 router 保持既有内嵌测试行为（直通）。
        #[tokio::test]
        async fn api_auth_unarmed_router_keeps_legacy_test_behavior() {
            let root = tempdir().expect("root");
            let app = build_web_router(state(root.path()));

            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/api/tasks")
                        .body(Body::empty())
                        .expect("legacy request"),
                )
                .await
                .expect("legacy response");
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "未装配认证时内嵌测试直通"
            );

            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/api/auth/session")
                        .body(Body::empty())
                        .expect("session check"),
                )
                .await
                .expect("session response");
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "未装配认证时会话端点不可用"
            );
        }
    }
}
