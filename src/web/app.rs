use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post};
use std::net::SocketAddr;
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
    let router = Router::new()
        .route("/api/health", get(handlers::health))
        .route("/api/providers/status", get(handlers::providers_status))
        .route("/api/providers/recheck", post(handlers::providers_recheck))
        .route("/api/runtime-info", get(handlers::runtime_info))
        .route(
            "/api/group-chat/sessions",
            post(handlers::group_chat_create_session),
        )
        .route(
            "/api/group-chat/sessions/{id}",
            get(handlers::group_chat_get_session),
        )
        .route(
            "/api/group-chat/sessions/{id}/messages",
            post(handlers::group_chat_send_message),
        )
        .route(
            "/api/group-chat/sessions/{id}/roles",
            post(handlers::group_chat_add_role),
        )
        .route(
            "/api/group-chat/sessions/{id}/finalize",
            post(handlers::group_chat_finalize),
        )
        .route(
            "/api/group-chat/sessions/{id}/settings/triage-provider",
            get(handlers::group_chat_get_triage_provider)
                .put(handlers::group_chat_update_triage_provider),
        )
        .route(
            "/api/settings/spec-generation-mode",
            get(handlers::get_spec_generation_mode)
                .put(handlers::update_spec_generation_mode),
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
            "/api/projects/{project_id}/repository-initializations/{operation_id}",
            get(handlers::get_repository_initialization),
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
            "/api/projects/{project_id}/issues/{issue_id}/work-item-plans/{plan_id}",
            delete(handlers::delete_work_item_plan),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/work-items/{work_item_id}",
            delete(handlers::delete_work_item),
        )
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/work-item-plans/{plan_id}/coding-attempts",
            post(handlers::create_group_coding_attempt),
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
    let product_paths = ProductAppPaths::new(workspace_root.join(".aria"));
    ensure_product_data_schema(&product_paths)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    let addr: SocketAddr = format!("{}:{}", host, port.unwrap_or(0)).parse()?;
    let events = EventHub::new();
    let state = WebAppState::with_events(
        workspace_root.clone(),
        crate::web::runtime::WebRuntime::new_real_with_events(workspace_root, events.clone())
            .map_err(|error| anyhow::anyhow!("{:?}: {}", error.code, error.message))?,
        events,
    );
    refresh_provider_health_for_startup(&state).await;
    let static_service = crate::web::static_assets::static_dist_service();
    let app = build_web_router(state).fallback(move |req: axum::extract::Request| {
        let static_service = static_service.clone();
        async move { crate::web::static_assets::serve_static(static_service, req).await }
    });
    let listener = TcpListener::bind(addr).await?;
    let bound_addr = listener.local_addr()?;
    eprintln!("{}", listening_line(&bound_addr));
    axum::serve(listener, app).await?;
    Ok(())
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

    use super::{build_web_router, refresh_provider_health_for_startup};
    use crate::cross_cutting::aria_state_paths::AriaStatePaths;
    use crate::cross_cutting::bounded_command_runner::{
        BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
    };
    use crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
    use crate::cross_cutting::provider_health::{ProviderHealthClock, ProviderHealthService};
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
}
