use axum::Router;
use axum::routing::{delete, get, post};
use std::net::SocketAddr;
use tokio::net::TcpListener;

use crate::web::coding_ws_handler;
use crate::web::events::EventHub;
use crate::web::handlers;
use crate::web::state::WebAppState;
use crate::web::test_controls;
use crate::web::workspace_ws_handler;

pub fn build_web_router(state: WebAppState) -> Router {
    let router = Router::new()
        .route("/api/health", get(handlers::health))
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
            "/api/projects/{project_id}/issues/{issue_id}/work-items:generate",
            post(handlers::generate_work_items),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/work-items/{work_item_id}",
            delete(handlers::delete_work_item),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/work-items/{work_item_id}/coding-attempts",
            post(handlers::create_coding_attempt),
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
        );

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
                "/api/test/workspace-sessions/large-fixture",
                post(test_controls::seed_large_workspace_fixture),
            )
            .route(
                "/api/test/permission-timeout",
                post(test_controls::set_permission_timeout),
            )
            .route("/api/test/ws-timeout", post(test_controls::set_ws_timeout))
    } else {
        router
    };

    router.with_state(state)
}

/// launcher 依赖的就绪行前缀契约。修改即破坏 launcher 解析，须同步更新 bin/aria.js 与回归测试。
pub const LISTENING_LINE_PREFIX: &str = "aria web listening on http://";

/// 生成就绪行（统一格式来源）。
pub fn listening_line(addr: &SocketAddr) -> String {
    format!("{LISTENING_LINE_PREFIX}{addr}")
}

pub async fn serve_web(
    workspace_root: std::path::PathBuf,
    host: String,
    port: Option<u16>,
) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("{}:{}", host, port.unwrap_or(0)).parse()?;
    let events = EventHub::new();
    let state = WebAppState::with_events(
        workspace_root.clone(),
        crate::web::runtime::WebRuntime::new_real_with_events(workspace_root, events.clone())
            .map_err(|error| anyhow::anyhow!("{:?}: {}", error.code, error.message))?,
        events,
    );
    let static_service = crate::web::static_assets::static_dist_service();
    let app = build_web_router(state).fallback(move |req: axum::extract::Request| {
        let static_service = static_service.clone();
        async move { crate::web::static_assets::serve_static(static_service, req).await }
    });
    crate::web::provider_probe::emit_provider_probe_notice();
    let listener = TcpListener::bind(addr).await?;
    let bound_addr = listener.local_addr()?;
    eprintln!("{}", listening_line(&bound_addr));
    axum::serve(listener, app).await?;
    Ok(())
}
