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
            "/api/projects/{project_id}/logical-codebase/initializations",
            post(handlers::create_aggregate_initialization),
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

    router.with_state(state)
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
) -> anyhow::Result<()> {
    let product_paths = ProductAppPaths::new(workspace_root.join(".aria"));
    ensure_product_data_schema(&product_paths)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    let addr: SocketAddr = format!("{}:{}", host, port.unwrap_or(0)).parse()?;
    let events = EventHub::new();
    let state = WebAppState::with_events(
        workspace_root.clone(),
        crate::web::runtime::WebRuntime::new_real_with_events(
            workspace_root.clone(),
            events.clone(),
        )
        .map_err(|error| anyhow::anyhow!("{:?}: {}", error.code, error.message))?,
        events,
    );
    refresh_provider_health_for_startup(&state).await;
    let static_service = crate::web::static_assets::static_dist_service();
    let evidence_enabled = is_loopback_host(&host);
    let app = build_web_router_with_evidence(state, evidence_enabled).fallback(
        move |req: axum::extract::Request| {
            let static_service = static_service.clone();
            async move { crate::web::static_assets::serve_static(static_service, req).await }
        },
    );
    let listener = TcpListener::bind(addr).await?;
    let bound_addr = listener.local_addr()?;
    maybe_write_web_endpoint_file(evidence_enabled, &workspace_root, bound_addr.port());
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
}
