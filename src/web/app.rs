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
        .route("/api/providers/status", get(handlers::providers_status))
        .route("/api/providers/recheck", post(handlers::providers_recheck))
        .route(
            "/api/providers/install-targets",
            get(handlers::get_install_targets),
        )
        .route(
            "/api/providers/{target}/install",
            post(handlers::post_provider_install),
        )
        .route(
            "/api/providers/install/{job_id}",
            get(handlers::get_install_job),
        )
        .route(
            "/api/providers/{target}/expose",
            post(handlers::post_provider_expose),
        )
        .route("/api/runtime-info", get(handlers::runtime_info))
        .route(
            "/api/providers/diagnostics",
            get(handlers::provider_diagnostics),
        )
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
        .route(
            "/api/image-create/settings/connectivity",
            post(handlers::probe_image_create_connectivity),
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
            post(handlers::create_logical_codebase).get(handlers::list_logical_codebases),
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
            "/api/projects/{project_id}/logical-codebases/{logical_codebase_id}/purge",
            delete(handlers::purge_logical_codebase),
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
            "/api/projects/{project_id}/logical-codebases/{lc_id}/reinitializations",
            post(handlers::create_lc_aggregate_reinitialization),
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
        // A（REQ-REG-10 自动推进）：五步顺序自动推进的显式入口（幂等）。
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/bootstrap/advance",
            post(handlers::post_lc_bootstrap_advance),
        )
        // LC 知识库模块（Task 7）：operation 生命周期 / 状态 / 文件面 / 变更包 /
        // 初始化输入模板，全部按 lc_id 作用域。
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/knowledge-base/initializations",
            post(handlers::knowledge_base::create_knowledge_base_initialization),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/knowledge-base/initializations/active",
            get(handlers::knowledge_base::get_active_knowledge_base_initialization),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/knowledge-base/initializations/{operation_id}",
            get(handlers::knowledge_base::get_knowledge_base_initialization),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/knowledge-base/initializations/{operation_id}/cancel",
            post(handlers::knowledge_base::cancel_knowledge_base_initialization),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/knowledge-base/status",
            get(handlers::knowledge_base::get_knowledge_base_status),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/knowledge-base/files",
            get(handlers::knowledge_base::list_knowledge_base_file_nodes),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/knowledge-base/file",
            get(handlers::knowledge_base::read_knowledge_base_file_content)
                .put(handlers::knowledge_base::write_knowledge_base_file_content),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/knowledge-base/change-packages",
            get(handlers::knowledge_base::list_knowledge_base_change_packages)
                .post(handlers::knowledge_base::create_knowledge_base_change_package),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/knowledge-base/change-packages/lint",
            get(handlers::knowledge_base::lint_knowledge_base_change_package),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/knowledge-base/bootstrap-inputs",
            post(handlers::knowledge_base::create_knowledge_base_bootstrap_inputs),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/knowledge-base/user-input",
            get(handlers::knowledge_base::get_knowledge_base_user_input),
        )
        .route(
            "/api/projects/{project_id}/logical-codebases/{lc_id}/knowledge-base/user-input/confirm",
            post(handlers::knowledge_base::confirm_knowledge_base_user_input),
        )
        // provider 能力面改**机器级**（REQ-PCR-08）：provider 是平台公用
        // 能力，与项目 / 逻辑代码库无关 ⇒ 旧的按项目 / 逻辑代码库定位的四条
        // 路由整条删除（不留兼容别名 / 转发 shim / 双写）。
        .route(
            "/api/providers/capabilities",
            get(handlers::get_lc_capabilities),
        )
        .route(
            "/api/providers/capabilities/revalidate",
            post(handlers::post_lc_capability_revalidate),
        )
        // 票3：「一键核验全部」异步作业（四家并发；POST 立即返回 job_id，GET 轮询）。
        .route(
            "/api/providers/capabilities/revalidate-all",
            post(handlers::post_lc_capability_revalidate_all),
        )
        .route(
            "/api/providers/capabilities/revalidate-all/{job_id}",
            get(handlers::get_lc_capability_revalidate_all_job),
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
            "/api/projects/{project_id}/issues/{issue_id}/candidate-attempt-metrics",
            get(handlers::get_issue_candidate_attempt_metrics),
        )
        .route(
            "/api/projects/{project_id}/candidate-attempt-metrics",
            get(handlers::get_project_candidate_attempt_metrics),
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
            "/api/coding-attempts/{attempt_id}/reviewer-window:resolve",
            post(handlers::resolve_reviewer_window_manual_gate),
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

    // 来源校验中间件挂在最外层——晚于全部 route 注册、先于任何
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

/// 生产 router 装配——静态 fallback 在内、来源校验层最外。
///
/// axum 0.8 中 `layer` 之后挂的 routes/fallback 不经该 middleware：把静态
/// fallback 挂在来源校验层之外时，未注册的 `/api/**`（如非回环未挂载的
/// evidence-query、任意拼错路径）会不经 Host/Origin 卫生落 SPA。故生产
/// 序固定为：业务 router（内层校验，供 oneshot 测试直用）→ 挂静态 fallback →
/// 再包最外层 `enforce_api_auth`（非 `/api`、`/ws` 路径直通，静态资源/SPA 公开）。
pub fn assemble_production_router(
    state: WebAppState,
    evidence_enabled: bool,
    static_service: crate::web::static_assets::StaticDistService,
) -> Router {
    build_web_router_with_evidence(state.clone(), evidence_enabled)
        .fallback(move |req: axum::extract::Request| {
            let static_service = static_service.clone();
            async move { crate::web::static_assets::serve_static(static_service, req).await }
        })
        // 认证层必须在 fallback 之外（最外层）：见本函数文档注释。
        .layer(axum::middleware::from_fn_with_state(
            state,
            crate::web::api_auth::enforce_api_auth,
        ))
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
    let static_service = crate::web::static_assets::static_dist_service();
    let evidence_enabled = is_loopback_host(&host);

    // 来源校验:先 bind 拿真实端口,再装配受信 Origin/Host 允许列表(默认精确
    // 来自该监听地址;bind 非 loopback 时必须显式配置 ARIA_API_TRUSTED_ORIGINS)。
    // B2/F3(批2 Bug45):bind 必须先于一切启动副作用——同端口 bind 失败
    // (端口被占)时零副作用返回,不得破坏其他实例正在持有的 reviewer
    // 窗口/租约/登记资源;补偿/分诊挪到 bind+来源校验装配之后(见下)。
    let listener = TcpListener::bind(addr).await?;
    let bound_addr = listener.local_addr()?;
    let explicit_origins = crate::web::api_auth::parse_trusted_origins_env(
        &std::env::var(crate::web::api_auth::TRUSTED_ORIGINS_ENV).unwrap_or_default(),
    );
    let auth = std::sync::Arc::new(
        crate::web::api_auth::ApiAuthState::for_bind(&bound_addr, &explicit_origins)
            .map_err(|error| anyhow::anyhow!("api auth bootstrap failed: {error}"))?,
    );
    let state = state.with_api_auth(auth);

    // T21:bind 成功且来源校验装配完成后,先扫描未关闭 reviewer 窗口并补偿
    // (停旧登记资源→按原 manifest 续恢复→人工门/释放),再放行
    // claimed-run/autopilot 分诊与 Provider 探活——不得沿旧序仅日志后
    // 继续启动同树 runner。失败 fail-visible,不阻塞服务(该树由租约
    // 闸门 fail-closed)。B2/F3:整块 reconcile+探活从 bind 之前挪到
    // bind/装配之后——同端口 bind 失败=零副作用;补偿自身另有租约
    // 闸门,owner 存活时零副作用跳过,不同端口并存实例互不破坏。
    if let Err(error) =
        crate::web::autopilot_orchestrator::reconcile_claimed_coding_runs_once(&state).await
    {
        eprintln!("[aria-startup] coding run startup reconcile failed: {error}");
    }
    // REQ-PRB-07：一次性孤儿 scratch 回收（不增常驻任务）。位置与既有
    // reconcile 同侧——bind 成功之后（B2/F3：bind 失败=零副作用）。失败只
    // 打日志、不阻塞启动；单项失败由 sweep 内部处理。
    if let Err(error) = crate::product::lifecycle_store::LifecycleStore::new(product_paths.clone())
        .sweep_orphan_run_scratch(
            crate::product::lifecycle_store::run_scratch::RUN_SCRATCH_ORPHAN_TTL,
        )
    {
        eprintln!("[aria-startup] orphan run scratch sweep failed: {error}");
    }
    // 悬空引用巡检 + 自动销账（owner 拍板：**服务启动时自动跑一次**，不做人工
    // 触发；`aria inspect dangling-references` 人工入口已删除）。位置与既有
    // reconcile / scratch sweep 同侧——bind 成功之后（B2/F3：bind 失败=零副作用）。
    // 删除判据仍是「白名单类型 + 证据达标」，不放宽；失败只打日志、**不阻断启动**
    // （巡检清单仍会写出，供事后查）。
    match crate::product::dangling_reference::startup::run_startup_governance(
        &product_paths,
        &workspace_root,
    ) {
        Ok(outcome) => {
            eprintln!(
                "[aria-startup] dangling reference governance done: report={} dangling={} undetermined={} deleted={} token={}",
                outcome.report_path.display(),
                outcome.dangling_count,
                outcome.undetermined_count,
                outcome.deleted_count,
                outcome.findings_token
            );
            // REQ-DRG-17 / 23：未归零可删项与不可判定项 MUST 逐条列出（不只打计数）。
            for line in outcome.log_lines() {
                eprintln!("{line}");
            }
        }
        Err(error) => eprintln!("[aria-startup] dangling reference governance failed: {error}"),
    }
    // 计划 2026-11-13 包 2.4b/2.4c/2.12：登记用户工具目录 + 异步探测登录 shell PATH/env。
    crate::cross_cutting::path_overlay::register_startup_overlay().await;
    refresh_provider_health_for_startup(&state).await;

    let app = assemble_production_router(state.clone(), evidence_enabled, static_service);
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

    /// 控制面来源校验真实 router 测试（owner 2026-10-22 决定后口径：无会话
    /// 放行、Host/Origin 前置、零副作用、真实 WS 握手、evidence 专用 token
    /// 语义、未装配直通）。
    mod api_auth_gates {
        use axum::body::Body;
        use axum::http::{HeaderValue, Request, StatusCode};
        use serde_json::json;
        use tempfile::tempdir;
        use tower::ServiceExt;

        use super::super::build_web_router;
        use super::super::build_web_router_with_evidence;
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

        fn armed_state(root: &std::path::Path) -> WebAppState {
            let bind: std::net::SocketAddr =
                format!("127.0.0.1:{AUTH_PORT}").parse().expect("bind addr");
            let auth = std::sync::Arc::new(ApiAuthState::for_bind(&bind, &[]).expect("auth"));
            state(root).with_api_auth(auth)
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

        /// 无会话放行（owner 2026-10-22）：受信 Host+Origin 下不带任何 cookie，
        /// mutation/敏感 GET/三类 WS（含 alias）全部穿过来源门进入 handler
        /// （不再有会话门）；`GET /api/health` 公开。
        #[tokio::test]
        async fn api_auth_allows_non_health_routes_without_session() {
            let root = tempdir().expect("root");
            let st = armed_state(root.path());
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
            }
            let cases = [
                Case {
                    method: "GET",
                    uri: "/api/providers/status",
                    ws: false,
                },
                Case {
                    method: "GET",
                    uri: "/api/runtime-info",
                    ws: false,
                },
                Case {
                    method: "GET",
                    uri: "/api/tasks",
                    ws: false,
                },
                Case {
                    method: "GET",
                    uri: "/api/workspaces",
                    ws: false,
                },
                Case {
                    method: "POST",
                    uri: "/api/providers/recheck",
                    ws: false,
                },
                Case {
                    method: "DELETE",
                    uri: "/api/workspaces/ws_0001",
                    ws: false,
                },
                Case {
                    method: "GET",
                    uri: "/api/workspace-sessions/sess_1/ws",
                    ws: true,
                },
                Case {
                    method: "GET",
                    uri: "/api/ws/workspace/sess_1",
                    ws: true,
                },
                Case {
                    method: "GET",
                    uri: "/ws/coding-attempts/attempt_1",
                    ws: true,
                },
                Case {
                    method: "GET",
                    uri: "/ws/projects/project_1/issues/issue_1/coding-attempts/attempt_1",
                    ws: true,
                },
                Case {
                    method: "GET",
                    uri: "/api/image-create/sessions/img_1/chat",
                    ws: true,
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
                    .oneshot(builder.body(Body::empty()).expect("request"))
                    .await
                    .expect("response");
                let status = response.status();
                assert_ne!(
                    status,
                    StatusCode::UNAUTHORIZED,
                    "{} {} 无会话不得被拒（无会话门）",
                    case.method,
                    case.uri
                );
                assert_ne!(
                    status,
                    StatusCode::FORBIDDEN,
                    "{} {} 受信来源不得被拒",
                    case.method,
                    case.uri
                );
            }
        }

        /// Host/Origin 前置：mutation/WS 必须受信精确 Origin，敏感 GET 无 Origin
        /// 放行；非法/null Origin、非法 Host → 403（无会话要求）。
        #[tokio::test]
        async fn api_auth_enforces_host_and_origin_before_handlers() {
            let root = tempdir().expect("root");
            let st = armed_state(root.path());
            let app = build_web_router(st);

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
                    if ws {
                        builder = ws_headers(builder);
                    }
                    builder.body(Body::empty()).expect("request")
                };

            // 敏感 GET：无 Origin 仍放行 + Host 正确 → 200。
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

            // loopback Host 变体（带受信端口）→ 200；裸 host（不带端口）属 403 名单
            // （上面的 bad_host 循环），回环口派生的 authority 只收 `host:port`。
            let response = app
                .clone()
                .oneshot(request(None, Some("localhost:39721"), "/api/tasks", false))
                .await
                .expect("loopback variant GET");
            assert_eq!(response.status(), StatusCode::OK);

            // mutation：缺 Origin → 403；恶意 Origin → 403；受信 → 直达 handler。
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/tasks")
                        .header("host", trusted_host())
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
                        .header("content-type", "application/json")
                        .body(valid_task_body())
                        .expect("mutation trusted"),
                )
                .await
                .expect("mutation response");
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "受信来源 mutation 无会话直达 handler"
            );

            // WS：无 Origin / 恶意 Origin → 403；受信 Origin → 穿过来源门。
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

        /// 403 拒绝零副作用：恶意 Origin 的 mutation 被拒且不创建对象；受信
        /// 同请求成功创建（探针有效）。
        #[tokio::test]
        async fn api_auth_rejection_produces_zero_side_effects() {
            let root = tempdir().expect("root");
            let st = armed_state(root.path());
            let app = build_web_router(st);

            // 403：恶意 Origin 的 mutation。
            let rejected = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/tasks")
                        .header("host", trusted_host())
                        .header("origin", "http://evil.example")
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
                        .body(Body::empty())
                        .expect("list"),
                )
                .await
                .expect("list response");
            assert_eq!(list_after_403.status(), StatusCode::OK);
            let payload: serde_json::Value =
                serde_json::from_str(&body_text(list_after_403).await).expect("json");
            assert_eq!(
                payload["tasks"].as_array().map(Vec::len),
                Some(0),
                "被拒请求不得产生副作用"
            );

            // 对照组：受信 Origin 的同请求创建 1 个任务（探针有效）。
            let created = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/tasks")
                        .header("host", trusted_host())
                        .header("origin", trusted_origin())
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
                        .body(Body::empty())
                        .expect("list"),
                )
                .await
                .expect("list response");
            let payload: serde_json::Value =
                serde_json::from_str(&body_text(list_after_create).await).expect("json");
            assert_eq!(payload["tasks"].as_array().map(Vec::len), Some(1));
        }

        /// 真实 TCP WS 握手：Origin 校验在 upgrade 前强制；受信 Origin 无任何
        /// cookie 即完成握手（101）；alias 同样受 Origin 保护。
        #[tokio::test]
        async fn api_auth_ws_handshake_enforced_on_real_server() {
            let root = tempdir().expect("root");
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind");
            let addr = listener.local_addr().expect("addr");
            let auth = std::sync::Arc::new(ApiAuthState::for_bind(&addr, &[]).expect("auth"));
            let app = build_web_router(state(root.path()).with_api_auth(auth));
            let server_app = app.clone();
            tokio::spawn(async move {
                axum::serve(listener, server_app).await.expect("serve");
            });

            let host = format!("127.0.0.1:{}", addr.port());
            let origin = format!("http://{host}");
            let ws_url = format!("ws://{host}/api/workspace-sessions/sess_ws1/ws");
            let alias_url = format!("ws://{host}/api/ws/workspace/sess_ws1");

            let request_with = |url: &str, origin: Option<String>| {
                use tokio_tungstenite::tungstenite::client::IntoClientRequest;
                let mut request = url.into_client_request().expect("client request");
                if let Some(origin) = origin {
                    request
                        .headers_mut()
                        .insert("origin", HeaderValue::from_str(&origin).expect("origin"));
                }
                request
            };

            // 无 Origin → 握手拒绝（403）。
            let no_origin = request_with(&ws_url, None);
            assert!(
                tokio_tungstenite::connect_async(no_origin).await.is_err(),
                "无 Origin 的 WS upgrade 必须被拒"
            );

            // 恶意 Origin → 握手拒绝（403）。
            let evil = request_with(&ws_url, Some("http://evil.example".to_string()));
            assert!(
                tokio_tungstenite::connect_async(evil).await.is_err(),
                "恶意 Origin 的 WS upgrade 必须被拒"
            );

            // 受信 Origin、无任何 cookie → 握手成功（101）。
            let allowed = request_with(&ws_url, Some(origin.clone()));
            let (stream, response) = tokio_tungstenite::connect_async(allowed)
                .await
                .expect("trusted-origin handshake must succeed");
            assert_eq!(response.status().as_u16(), 101, "upgrade 必须完成");
            drop(stream);

            // alias 路径同样受 Origin 校验保护：缺 Origin 拒绝、受信成功。
            let alias_no_origin = request_with(&alias_url, None);
            assert!(
                tokio_tungstenite::connect_async(alias_no_origin)
                    .await
                    .is_err(),
                "alias WS 缺 Origin 必须被拒"
            );
            let alias_allowed = request_with(&alias_url, Some(origin.clone()));
            let (stream, response) = tokio_tungstenite::connect_async(alias_allowed)
                .await
                .expect("alias trusted-origin handshake must succeed");
            assert_eq!(response.status().as_u16(), 101);
            drop(stream);
        }

        /// 未装配 api_auth 的 router 保持既有内嵌测试行为（直通）；已删除的
        /// `/api/auth/session` 路由不再注册（404）。
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
                "未装配来源校验时内嵌测试直通"
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
                StatusCode::NOT_FOUND,
                "会话/token 端点已删除"
            );
        }

        /// 批2 T1 修复轮 2（NEW-2 裁定）：/api/evidence-query 走专用 evidence
        /// token 语义——合法专用 token 穿过中间件进入 handler 证据链；非法
        /// token 由 handler 专用链拒绝；Host/Origin 卫生照常。
        #[tokio::test]
        async fn api_auth_evidence_query_uses_dedicated_token_semantics() {
            let root = tempdir().expect("root");

            // 专用 token fixture：attempt 记录 + issue_evidence_token（与
            // handlers/evidence_query.rs 测试同构的最小链）。
            let paths = crate::product::app_paths::ProductAppPaths::new(root.path().join(".aria"));
            let repo = root.path().join("repo");
            std::fs::create_dir_all(&repo).expect("repo dir");
            let attempt = crate::product::coding_models::CodingExecutionAttempt {
                id: "coding_attempt_0001".to_string(),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                work_item_id: "work_item_0001".to_string(),
                attempt_no: 1,
                scope: crate::product::coding_models::CodingAttemptScope::WorkItem,
                status: crate::product::coding_models::CodingAttemptStatus::Running,
                version: 0,
                manual_recovery_reason: None,
                admission_ticket_consumed_at: None,
                admission_kind: crate::product::coding_models::CodingAdmissionKind::LegacyGroup,
                stage: crate::product::coding_models::CodingExecutionStage::Coding,
                base_branch: "main".to_string(),
                branch_name: "aria/issues/issue_0001".to_string(),
                worktree_path: None,
                provider_config_snapshot: crate::web::workspace_ws_types::ProviderConfigSnapshot {
                    author: crate::product::models::ProviderName::Fake,
                    reviewer: None,
                    review_rounds: 0,
                    permission_modes: Default::default(),
                },
                rework_count: 0,
                max_auto_rework: 0,
                work_item_group_id: None,
                current_work_item_id: Some("work_item_0001".to_string()),
                active_unit_id: None,
                head_commit: None,
                pushed_remote: None,
                review_request_id: None,
                provider_conversations: Vec::new(),
                created_at: "2026-08-11T00:00:00Z".to_string(),
                updated_at: "2026-08-11T00:00:00Z".to_string(),
                target_snapshot: None,
                start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
                start_claim: None,
                completed_at: None,
            };
            crate::product::json_store::write_json(
                &paths
                    .issue_lifecycle_root("project_0001", "issue_0001")
                    .join("coding-attempts")
                    .join("coding_attempt_0001.json"),
                &attempt,
            )
            .expect("write attempt record");
            let token = crate::product::logical_codebase::evidence_token::issue_evidence_token(
                &paths, &repo, &attempt,
            )
            .expect("issue evidence token");

            // 来源校验开启的真实 evidence 路由（evidence_enabled=true 挂载）。
            let bind: std::net::SocketAddr =
                format!("127.0.0.1:{AUTH_PORT}").parse().expect("bind");
            let auth = std::sync::Arc::new(
                crate::web::api_auth::ApiAuthState::for_bind(&bind, &[]).expect("auth"),
            );
            let app = build_web_router_with_evidence(state(root.path()).with_api_auth(auth), true);

            let dedicated_token_request =
                |host: Option<&str>, origin: Option<&str>, token: &str| {
                    let mut builder = Request::builder()
                        .method("POST")
                        .uri("/api/evidence-query")
                        .header("content-type", "application/json");
                    if let Some(host) = host {
                        builder = builder.header("host", host);
                    }
                    if let Some(origin) = origin {
                        builder = builder.header("origin", origin);
                    }
                    builder
                        .body(Body::from(
                            json!({ "token": token, "role": "coder", "query": "some_symbol" })
                                .to_string(),
                        ))
                        .expect("evidence request")
                };
            let response_code = |response: axum::response::Response| async move {
                let text = body_text(response).await;
                serde_json::from_str::<serde_json::Value>(&text)
                    .expect("handler-domain JSON body")
                    .get("code")
                    .and_then(|code| code.as_str())
                    .expect("code field")
                    .to_string()
            };

            // 1) 合法专用 token → 穿过中间件进入 handler 证据链（无 target
            //    snapshot → handler 域结果 evidence_not_available，不是 403）。
            let response = app
                .clone()
                .oneshot(dedicated_token_request(
                    Some(trusted_host()),
                    Some(trusted_origin()),
                    &token,
                ))
                .await
                .expect("valid token response");
            assert_ne!(
                response.status(),
                StatusCode::FORBIDDEN,
                "Origin/Host 卫生不应拦截受信来源"
            );
            let code = response_code(response).await;
            assert_eq!(
                code, "evidence_not_available",
                "合法 token 必须由 handler 证据链应答"
            );

            // 2) 非法 token → handler 专用链拒绝（evidence_unauthorized）。
            let response = app
                .clone()
                .oneshot(dedicated_token_request(
                    Some(trusted_host()),
                    Some(trusted_origin()),
                    "deadbeef",
                ))
                .await
                .expect("invalid token response");
            let code = response_code(response).await;
            assert_eq!(
                code, "evidence_unauthorized",
                "非法 token 由专用 token 链拒绝"
            );

            // 3) 缺 Host / 伪造 Host → 中间件 403（先于专用链）。
            let response = app
                .clone()
                .oneshot(dedicated_token_request(
                    None,
                    Some(trusted_origin()),
                    &token,
                ))
                .await
                .expect("missing host response");
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            let response = app
                .clone()
                .oneshot(dedicated_token_request(
                    Some("evil.example:4317"),
                    Some(trusted_origin()),
                    &token,
                ))
                .await
                .expect("evil host response");
            assert_eq!(response.status(), StatusCode::FORBIDDEN);

            // 4) 恶意 / null Origin → 中间件 403（mutation 来源卫生保留）。
            for evil in ["http://evil.example", "null"] {
                let response = app
                    .clone()
                    .oneshot(dedicated_token_request(
                        Some(trusted_host()),
                        Some(evil),
                        &token,
                    ))
                    .await
                    .expect("evil origin response");
                assert_eq!(
                    response.status(),
                    StatusCode::FORBIDDEN,
                    "Origin={evil} 须 403"
                );
            }
        }

        /// 专用 token 豁免镜像挂载条件（仅回环）：非回环 bind 不挂载 evidence
        /// 路由，其 Host 下的 evidence-query 与任意未注册 `/api/**` 路径都
        /// 不得命中 evidence handler（落静态 fallback，绝非 handler 域 JSON）；
        /// 非 /api 前缀路径（静态资源/SPA）不受来源校验层影响。
        #[tokio::test]
        async fn api_auth_evidence_query_exemption_limited_to_loopback() {
            let root = tempdir().expect("root");
            let bind: std::net::SocketAddr = "192.168.1.5:4317".parse().expect("bind");
            let auth = std::sync::Arc::new(
                crate::web::api_auth::ApiAuthState::for_bind(
                    &bind,
                    &["http://aria.example:4317".to_string()],
                )
                .expect("non-loopback auth with explicit origin"),
            );
            // 生产同序装配：evidence_enabled=false（非回环不挂载）+ 静态
            // fallback + 最外层来源校验。
            let static_service = crate::web::static_assets::static_dist_service();
            let app = super::super::assemble_production_router(
                state(root.path()).with_api_auth(auth),
                false,
                static_service,
            );

            // 1) 非回环 Host 的 POST /api/evidence-query：受信来源穿过来源门，
            //    但路由未挂载 → 落静态 fallback；绝不能命中 evidence handler
            //    域 JSON（evidence_unauthorized / evidence_not_available）。
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/evidence-query")
                        .header("host", "aria.example:4317")
                        .header("origin", "http://aria.example:4317")
                        .header("content-type", "application/json")
                        .body(Body::from(
                            json!({ "token": "deadbeef", "role": "coder", "query": "x" })
                                .to_string(),
                        ))
                        .expect("non-loopback evidence request"),
                )
                .await
                .expect("response");
            assert_ne!(
                response.status(),
                StatusCode::FORBIDDEN,
                "受信来源不应被来源门拒绝"
            );
            let body = body_text(response).await;
            assert!(
                !body.contains("evidence_unauthorized") && !body.contains("evidence_not_available"),
                "非回环 bind 未挂载 evidence 路由，不得命中 evidence handler：{body}"
            );

            // 2) 任意未注册 /api/** 路径同样落静态（无会话门，来源卫生照常）。
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/api/not-a-registered-route")
                        .header("host", "aria.example:4317")
                        .body(Body::empty())
                        .expect("unregistered api request"),
                )
                .await
                .expect("unregistered api response");
            assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
            assert_ne!(response.status(), StatusCode::FORBIDDEN);

            // 3) 非 /api、/ws 前缀路径直通来源校验层（静态资源/SPA 可达不受
            //    影响；嵌入资源命中与否取决于构建，但绝不能被来源层 401/403
            //    拦截）。
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/")
                        .body(Body::empty())
                        .expect("spa request"),
                )
                .await
                .expect("spa response");
            assert_ne!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "静态/SPA 路径不受来源校验层影响"
            );
            assert_ne!(response.status(), StatusCode::FORBIDDEN);
        }
    }

    /// REQ-PRB-07：启动 sweep 必须在 bind 之后、在开始 serve 之前执行一次，
    /// 失败只打日志不得传播。计划原文按单行调用字面量断言；该调用含全路径
    /// 具名 TTL，rustfmt（max_width=100）必然折行，故改为按位置、形参与
    /// 失败处理方式断言。
    #[test]
    fn orphan_run_scratch_sweep_runs_after_bind() {
        let source = include_str!("app.rs");
        let start = source.find("pub async fn serve_web(").expect("serve_web");
        let end = start
            + source[start..]
                .find("fn orchestrator_wake_subscribe")
                .expect("serve_web end");
        let body = &source[start..end];

        let bind = body.find("TcpListener::bind(addr)").expect("bind call");
        let reconcile = body
            .find("reconcile_claimed_coding_runs_once")
            .expect("coding run reconcile");
        let sweep = body
            .find("sweep_orphan_run_scratch(")
            .expect("orphan run scratch sweep call");
        let serve = body.find("axum::serve(listener, app)").expect("serve call");

        assert!(
            bind < sweep,
            "sweep 必须在 bind 之后（B2/F3：bind 失败=零副作用）"
        );
        assert!(
            reconcile < sweep,
            "sweep 必须与既有启动副作用同侧（reconcile 之后）"
        );
        assert!(sweep < serve, "sweep 必须在开始 serve 之前执行");
        assert_eq!(
            body.matches("sweep_orphan_run_scratch(").count(),
            1,
            "启动 sweep 只执行一次，不得是周期任务"
        );

        // 失败处理：`if let Err(error) = …sweep_orphan_run_scratch(…) { 日志 }`，
        // 不得用 `?` 把错误传播成启动失败。
        let guard = &body[sweep.saturating_sub(300)..sweep];
        assert!(
            guard.contains("if let Err(error)"),
            "sweep 结果必须显式捕获：{guard}"
        );
        assert!(
            guard.contains("product_paths"),
            "sweep 必须使用 serve_web 内的 product_paths：{guard}"
        );
        let logged = body[sweep..]
            .find("orphan run scratch sweep failed")
            .expect("sweep 失败必须只打日志");
        let call = &body[sweep..sweep + logged];
        assert!(
            call.contains("run_scratch::RUN_SCRATCH_ORPHAN_TTL"),
            "TTL 必须传具名常量 RUN_SCRATCH_ORPHAN_TTL：{call}"
        );
        assert!(!call.contains("from_secs"), "TTL 不得写字面量时长：{call}");
        assert!(!call.contains('?'), "sweep 失败不得用 ? 传播：{call}");
    }

    /// 悬空引用治理（巡检 + 自动销账）必须在 bind 之后、开始 serve 之前执行
    /// **一次**，且失败只打日志、**不得阻断启动**（owner 拍板：服务启动时自动
    /// 跑一次，人工 CLI 入口已删除）。
    #[test]
    fn dangling_reference_governance_runs_once_after_bind_and_never_blocks_startup() {
        let source = include_str!("app.rs");
        let start = source.find("pub async fn serve_web(").expect("serve_web");
        let end = start
            + source[start..]
                .find("fn orchestrator_wake_subscribe")
                .expect("serve_web end");
        let body = &source[start..end];

        let bind = body.find("TcpListener::bind(addr)").expect("bind call");
        let sweep = body
            .find("sweep_orphan_run_scratch(")
            .expect("orphan run scratch sweep call");
        let governance = body
            .find("run_startup_governance(")
            .expect("dangling reference governance call");
        let serve = body.find("axum::serve(listener, app)").expect("serve call");

        assert!(
            bind < governance,
            "治理必须在 bind 之后（B2/F3：bind 失败=零副作用）"
        );
        assert!(
            sweep < governance,
            "治理必须与既有启动副作用同侧（scratch sweep 之后）"
        );
        assert!(governance < serve, "治理必须在开始 serve 之前执行");
        assert_eq!(
            body.matches("run_startup_governance(").count(),
            1,
            "启动治理只执行一次，不得是周期任务"
        );

        // 失败处理：必须显式捕获并打日志，不得用 `?` 把错误传播成启动失败。
        let match_call = body
            .find("match crate::product::dangling_reference::startup::run_startup_governance(")
            .expect("治理必须以 match 显式捕获结果");
        assert!(match_call < governance, "治理必须以 match 显式捕获结果");
        let guard = &body[match_call..(match_call + 200).min(body.len())];
        assert!(
            guard.contains("product_paths"),
            "治理必须使用 serve_web 内的 product_paths：{guard}"
        );
        assert!(
            guard.contains("workspace_root"),
            "治理必须使用 serve_web 内的 workspace_root（清单落点）：{guard}"
        );
        let logged = body[governance..]
            .find("dangling reference governance failed")
            .expect("治理失败必须只打日志");
        let call = &body[governance..governance + logged];
        assert!(!call.contains('?'), "治理失败不得用 ? 传播：{call}");
    }
}
