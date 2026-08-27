//! 集成测试入口：web 域。各子模块原为独立 tests/*.rs，合并以减少二进制数量。
use tower::ServiceExt;

static TEST_CONTROLS_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) struct TestControlsEnvGuard {
    _guard: tokio::sync::MutexGuard<'static, ()>,
}

impl Drop for TestControlsEnvGuard {
    fn drop(&mut self) {
        unsafe {
            std::env::remove_var("ARIA_E2E_TEST_CONTROLS");
        }
    }
}

pub(crate) async fn enable_test_controls() -> TestControlsEnvGuard {
    let guard = TEST_CONTROLS_ENV_LOCK.lock().await;
    unsafe {
        std::env::set_var("ARIA_E2E_TEST_CONTROLS", "1");
    }
    TestControlsEnvGuard { _guard: guard }
}

pub(crate) async fn disable_test_controls() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = TEST_CONTROLS_ENV_LOCK.lock().await;
    unsafe {
        std::env::remove_var("ARIA_E2E_TEST_CONTROLS");
    }
    guard
}

pub(crate) async fn create_repository_and_wait(
    app: axum::Router,
    project_id: &str,
    request_body: serde_json::Value,
) -> serde_json::Value {
    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method(axum::http::Method::POST)
                .uri(format!("/api/projects/{project_id}/repositories"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(request_body.to_string()))
                .expect("create repository request"),
        )
        .await
        .expect("create repository response");
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("create repository body");
    let accepted: serde_json::Value = serde_json::from_slice(&body).expect("accepted operation");
    assert_eq!(
        status,
        axum::http::StatusCode::ACCEPTED,
        "repository initialization must be asynchronous: {accepted}"
    );
    let operation_id = accepted["operation_id"]
        .as_str()
        .expect("accepted repository initialization operation id");
    let operation_uri =
        format!("/api/projects/{project_id}/repository-initializations/{operation_id}");
    let mut last_snapshot = accepted;
    for _ in 0..100 {
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method(axum::http::Method::GET)
                    .uri(&operation_uri)
                    .body(axum::body::Body::empty())
                    .expect("repository initialization status request"),
            )
            .await
            .expect("repository initialization status response");
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("repository initialization status body");
        let snapshot: serde_json::Value =
            serde_json::from_slice(&body).expect("repository initialization status json");
        assert_eq!(
            status,
            axum::http::StatusCode::OK,
            "repository initialization status request failed: {snapshot}"
        );
        match snapshot["status"].as_str() {
            Some("completed") => {
                let result = snapshot["result"].clone();
                assert!(
                    result.is_object(),
                    "completed repository initialization result is missing: {snapshot}"
                );
                return result;
            }
            Some("failed") => panic!("repository initialization failed: {snapshot}"),
            _ => {
                last_snapshot = snapshot;
                tokio::task::yield_now().await;
            }
        }
    }
    panic!("repository initialization did not reach a terminal state: {last_snapshot}");
}

#[path = "it_web/evidence_query.rs"]
mod evidence_query;
#[path = "it_web/issue_delivery_summary.rs"]
mod issue_delivery_summary;
#[path = "it_web/pointer_publication.rs"]
mod pointer_publication;
#[path = "it_web/provider_gateway_envelope.rs"]
mod provider_gateway_envelope;
#[path = "it_web/web_api_handlers.rs"]
mod web_api_handlers;
#[path = "it_web/web_cli.rs"]
mod web_cli;
#[path = "it_web/web_codebases_api.rs"]
mod web_codebases_api;
#[path = "it_web/web_coding_attempt_api.rs"]
mod web_coding_attempt_api;
#[path = "it_web/web_coding_ws_handler.rs"]
mod web_coding_ws_handler;
#[path = "it_web/web_event_taxonomy.rs"]
mod web_event_taxonomy;
#[path = "it_web/web_events.rs"]
mod web_events;
#[path = "it_web/web_hard_gate.rs"]
mod web_hard_gate;
#[path = "it_web/web_lc_operations_api.rs"]
mod web_lc_operations_api;
#[path = "it_web/web_lc_registration_api.rs"]
mod web_lc_registration_api;
#[path = "it_web/web_lifecycle_api.rs"]
mod web_lifecycle_api;
#[path = "it_web/web_listening_line.rs"]
mod web_listening_line;
#[path = "it_web/web_node_context.rs"]
mod web_node_context;
#[path = "it_web/web_policy_runtime.rs"]
mod web_policy_runtime;
#[path = "it_web/web_product_api.rs"]
mod web_product_api;
#[path = "it_web/web_projection.rs"]
mod web_projection;
#[path = "it_web/web_provider_availability.rs"]
mod web_provider_availability;
#[path = "it_web/web_provider_execution_events.rs"]
mod web_provider_execution_events;
#[path = "it_web/web_provider_health_api.rs"]
mod web_provider_health_api;
#[path = "it_web/web_provider_output_events.rs"]
mod web_provider_output_events;
#[path = "it_web/web_provider_probe.rs"]
mod web_provider_probe;
#[path = "it_web/web_repository_initialization.rs"]
mod web_repository_initialization;
#[path = "it_web/web_resource_handlers.rs"]
mod web_resource_handlers;
#[path = "it_web/web_runtime_fake.rs"]
mod web_runtime_fake;
#[path = "it_web/web_runtime_persistence.rs"]
mod web_runtime_persistence;
#[path = "it_web/web_runtime_real.rs"]
mod web_runtime_real;
#[path = "it_web/web_static_assets.rs"]
mod web_static_assets;
#[path = "it_web/web_test_controls.rs"]
mod web_test_controls;
#[path = "it_web/web_types.rs"]
mod web_types;
#[path = "it_web/web_work_item_generation.rs"]
mod web_work_item_generation;
#[path = "it_web/web_work_item_plan_author.rs"]
mod web_work_item_plan_author;
#[path = "it_web/web_work_item_plan_batch.rs"]
mod web_work_item_plan_batch;
#[path = "it_web/web_work_item_plan_compile.rs"]
mod web_work_item_plan_compile;
#[path = "it_web/web_work_item_plan_confirm.rs"]
mod web_work_item_plan_confirm;
#[path = "it_web/web_work_item_plan_mode.rs"]
mod web_work_item_plan_mode;
#[path = "it_web/web_work_item_plan_outline.rs"]
mod web_work_item_plan_outline;
#[path = "it_web/web_work_item_plan_repair.rs"]
mod web_work_item_plan_repair;
#[path = "it_web/web_work_item_plan_revert.rs"]
mod web_work_item_plan_revert;
#[path = "it_web/web_work_item_plan_review.rs"]
mod web_work_item_plan_review;
#[path = "it_web/web_work_item_plan_serial.rs"]
mod web_work_item_plan_serial;
#[path = "it_web/web_work_item_plan_staged_flow.rs"]
mod web_work_item_plan_staged_flow;
#[path = "it_web/web_work_item_split_flow.rs"]
mod web_work_item_split_flow;
#[path = "it_web/web_workspace_recovery_consistency.rs"]
mod web_workspace_recovery_consistency;
#[path = "it_web/web_workspace_takeover_api.rs"]
mod web_workspace_takeover_api;

/// Fixture 初始状态播种：受控写 API 既不允许改 attempt 身份（id/issue），也不允许
/// 直接写 status；fixture 沿 store 路径直接落盘初始 record（等价于 lib 测试的
/// write_coding_attempt_for_test，仅用于集成测试初始状态，不代表生产写路径）。
pub(crate) fn seed_coding_attempt_record(
    store: &cadence_aria::product::coding_attempt_store::CodingAttemptStore,
    attempt: &cadence_aria::product::coding_models::CodingExecutionAttempt,
) {
    cadence_aria::product::json_store::write_json(
        &store
            .paths()
            .issue_root(&attempt.project_id, &attempt.issue_id)
            .join("coding-attempts")
            .join(format!("{}.json", attempt.id)),
        attempt,
    )
    .expect("seed coding attempt record fixture");
}

/// Fixture 初始状态播种：将 attempt 置为 Running 可执行态。先经 store 读取 record，
/// 置 status=Running 并写入 admission 会话标记，再经 `seed_coding_attempt_record`
/// 落盘——模拟「已经 admission CAS 进入」的合法可执行状态（状态机已封死直达
/// Running 通道；播种不经 `update_attempt_status`，仅用于集成测试初始状态，
/// 不代表生产写路径）。
pub(crate) fn seed_coding_attempt_running(
    store: &cadence_aria::product::coding_attempt_store::CodingAttemptStore,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
) -> cadence_aria::product::coding_models::CodingExecutionAttempt {
    let mut attempt = store
        .get_attempt(project_id, issue_id, attempt_id)
        .expect("load coding attempt for Running seeding");
    attempt.status = cadence_aria::product::coding_models::CodingAttemptStatus::Running;
    attempt.admission_ticket_consumed_at = Some(chrono::Utc::now().to_rfc3339());
    seed_coding_attempt_record(store, &attempt);
    attempt
}
