//! add-provider-revalidate-probe（Task 2）：capability 重验证 HTTP 面的
//! it_web 验收。
//!
//! - GET `/logical-codebases/{lc_id}/capabilities`：只读投影四家 durable
//!   capability 行（未探测全 Unknown；bootstrap 默认记录不显示 Confirmed）；
//!   GET 不触发任何探针/写入。
//! - POST `/logical-codebases/{lc_id}/capability-revalidate`：
//!   - Fake/未知 provider_type → 422 `provider_capability_probe_unsupported`，
//!    零探针、零 durable 写入；
//!   - 全行已 Confirmed@当前版本（注入版本源）→ 200 `already_confirmed`
//!     秒回（探针 runner 被调用即失败——证明不重跑）；
//!   - 探针失败（注入恒败 runner）→ 稳定码 `provider_capability_probe_failed`
//!     携带 action/detail，durable 保持点击前状态；
//!   - 未知 LC → 404 `logical_codebase_not_found`。
//! - 探测导入成功路径（probe→import→Confirmed→再点秒回）由产品层
//!   `provider_capability_revalidate` 单测以真实三方一致性材料包覆盖
//!   （it_web 侧 pub(crate) 构造器不可达，不在此重复）。

use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::cross_cutting::provider_boundary::ProviderBoundaryError;
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::logical_codebase::provider_capability_revalidate::{
    ProbeInvocation, ProviderCapabilityRevalidateService,
};
use cadence_aria::product::logical_codebase::store::{
    LogicalCodebaseCreateInput, LogicalCodebaseStore,
};
use cadence_aria::product::project_store::{CreateProjectInput, ProjectStore};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::events::EventHub;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::{Value, json};
use tempfile::tempdir;
use tower::ServiceExt;

const PROJECT_ID: &str = "project_0001";

async fn request_json(
    app: &axum::Router,
    method: Method,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let text = String::from_utf8_lossy(&bytes);
    let value = if text.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("non-JSON body: {text} ({error})"))
    };
    (status, value)
}

/// project + LC record 直建（capability 面只要求 LC 在场，不要求登记/
/// 初始化链完成）。
fn create_project_and_lc(paths: &ProductAppPaths, aggregate_root: &Path) -> String {
    ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "capability-project".to_string(),
            description: None,
        })
        .expect("project");
    std::fs::create_dir_all(aggregate_root).expect("aggregate root");
    LogicalCodebaseStore::new(paths.clone())
        .create(
            PROJECT_ID,
            LogicalCodebaseCreateInput {
                name: "capability-lc".to_string(),
                aggregate_root: aggregate_root.to_path_buf(),
            },
        )
        .expect("lc")
        .id
}

fn app_fixture(
    service_for_root: impl FnOnce(&Path) -> Option<Arc<ProviderCapabilityRevalidateService>>,
) -> (axum::Router, ProductAppPaths, String) {
    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lc_id = create_project_and_lc(&paths, &root.path().join("aggregate-root"));
    let mut state = WebAppState::with_events(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        EventHub::new(),
    );
    if let Some(service) = service_for_root(root.path()) {
        state = state.with_capability_revalidate_service(service);
    }
    let app = build_web_router(state);
    // manifest/记录事实须在 tempdir 存活期间使用——泄漏 root（既有 fixture 取舍）。
    std::mem::forget(root);
    (app, paths, lc_id)
}

fn capabilities_uri(lc_id: &str) -> String {
    format!("/api/projects/{PROJECT_ID}/logical-codebases/{lc_id}/capabilities")
}

fn revalidate_uri(lc_id: &str) -> String {
    format!("/api/projects/{PROJECT_ID}/logical-codebases/{lc_id}/capability-revalidate")
}

/// 注入固定版本源 + 「被调用即 panic」探针 runner（证明秒回不重跑）。
fn fast_path_service(
    aria_root: &std::path::Path,
    version: &'static str,
) -> Arc<ProviderCapabilityRevalidateService> {
    Arc::new(
        ProviderCapabilityRevalidateService::new(ProductAppPaths::new(aria_root.to_path_buf()))
            .with_version_source(Arc::new(move |_program: &str| Ok(version.to_string())))
            .with_probe_runner(Arc::new(|_invocation: ProbeInvocation| {
                Box::pin(async {
                    panic!("already_confirmed 秒回不得触发任何探针");
                })
            })),
    )
}

/// 注入恒败探针 runner（证明失败如实上报）。
fn failing_service(aria_root: &std::path::Path) -> Arc<ProviderCapabilityRevalidateService> {
    Arc::new(
        ProviderCapabilityRevalidateService::new(ProductAppPaths::new(aria_root.to_path_buf()))
            .with_version_source(Arc::new(|_program: &str| Ok("1.42.0".to_string())))
            .with_probe_runner(Arc::new(|invocation: ProbeInvocation| {
                let action = invocation.action;
                Box::pin(async move {
                    Err(ProviderBoundaryError::ProbeFailed(format!(
                        "boundary_unobserved: {action:?} write-surface probe channel failed"
                    )))
                })
            })),
    )
}

#[tokio::test]
async fn capabilities_get_projects_durable_rows_without_confirmed_defaults() {
    let (app, _paths, lc_id) = app_fixture(|_| None);
    let (status, body) =
        request_json(&app, Method::GET, &capabilities_uri(&lc_id), json!({})).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let providers = body["providers"].as_array().expect("providers");
    assert_eq!(providers.len(), 4, "四家真实 provider: {body}");
    let types: Vec<&str> = providers
        .iter()
        .map(|provider| provider["provider_type"].as_str().expect("type"))
        .collect();
    for expected in ["claude_code", "codex", "pi", "kimi_code"] {
        assert!(types.contains(&expected), "缺 {expected}: {types:?}");
    }
    for provider in providers {
        assert_eq!(provider["version"], Value::Null, "未探测无版本");
        for row in provider["rows"].as_array().expect("rows") {
            assert_eq!(row["launch"], json!("unknown"), "未探测全 Unknown: {row}");
            assert_eq!(row["resume"], json!("unknown"), "未探测全 Unknown: {row}");
            assert_eq!(
                row["write_boundary"],
                json!("unknown"),
                "未探测全 Unknown: {row}"
            );
        }
    }
    // 只读：重复 GET 投影不变（零写入）。
    let (_, body_again) =
        request_json(&app, Method::GET, &capabilities_uri(&lc_id), json!({})).await;
    assert_eq!(body, body_again, "GET 必须零副作用");
}

#[tokio::test]
async fn capability_revalidate_rejects_fake_provider_without_probe_or_write() {
    let (app, paths, lc_id) = app_fixture(|_| None);
    let (status, body) = request_json(
        &app,
        Method::POST,
        &revalidate_uri(&lc_id),
        json!({ "provider_type": "fake" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body: {body}");
    assert_eq!(body["code"], "provider_capability_probe_unsupported");
    // 零 durable 写入：capabilities.json 不因拒绝产生。
    let capabilities = paths
        .logical_codebases_root(PROJECT_ID)
        .join(&lc_id)
        .join("capabilities.json");
    assert!(
        !capabilities.exists(),
        "拒绝路径不得写 capability 记录: {}",
        capabilities.display()
    );
}

#[tokio::test]
async fn capability_revalidate_unknown_provider_type_is_unsupported() {
    let (app, _paths, lc_id) = app_fixture(|_| None);
    let (status, body) = request_json(
        &app,
        Method::POST,
        &revalidate_uri(&lc_id),
        json!({ "provider_type": "not-a-provider" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body: {body}");
    assert_eq!(body["code"], "provider_capability_probe_unsupported");
}

#[tokio::test]
async fn capability_revalidate_unknown_lc_is_404() {
    let (app, _paths, _lc_id) = app_fixture(|_| None);
    let (status, body) = request_json(
        &app,
        Method::POST,
        &revalidate_uri("lc_unknown"),
        json!({ "provider_type": "codex" }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert_eq!(body["code"], "logical_codebase_not_found");
}

/// fast-path：seed 全行 Confirmed@1.42.0（durable 直建，全 pub 材料包），
/// 注入同版本版本源 → POST 秒回 already_confirmed，探针 runner 被触发即
/// panic（`fast_path_service`）。
#[tokio::test]
async fn capability_revalidate_confirmed_rows_fast_return_without_probe() {
    // seed 由产品层 store 完成（pub API）。
    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lc_id = create_project_and_lc(&paths, &root.path().join("aggregate-root"));
    use cadence_aria::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
    use cadence_aria::product::logical_codebase::policy::{
        ProviderDialect, ProviderWireDialect, SessionPolicyAction,
    };
    use cadence_aria::product::logical_codebase::provider_capability_store::{
        CapabilityEvidence, PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION, ProviderActionCapability,
        ProviderActionMatrix, ProviderCapabilityRecord, ProviderCapabilityStore,
        RootRecipeEvidence,
    };
    use cadence_aria::product::logical_codebase::provider_gateway::{
        ProviderRefType, ResumeEvidenceState,
    };
    let rows = [
        SessionPolicyAction::CodingTargetWrite,
        SessionPolicyAction::PlanningReadOnly,
        SessionPolicyAction::ReviewReadOnly,
    ]
    .iter()
    .map(|action| ProviderActionCapability {
        action: *action,
        launch: ProviderCapabilityEvidence::Confirmed,
        resume: ProviderCapabilityEvidence::Confirmed,
        write_boundary: ProviderCapabilityEvidence::Confirmed,
        projection_digest: format!("sha256:{}", "7".repeat(64)),
        evidence_ref: "probe://boundary/codex/seed".to_string(),
    })
    .collect();
    ProviderCapabilityStore::for_lc(paths.clone(), &lc_id)
        .upsert(
            PROJECT_ID,
            &ProviderCapabilityRecord {
                provider_type: ProviderRefType::Codex,
                schema_version: PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION,
                version: "1.42.0".to_string(),
                adapter_dialect: ProviderDialect::CodexCliV1,
                wire_dialect: ProviderWireDialect::CodexAppServerRpc,
                capability_snapshot_ref: "cap_managed_snapshot".to_string(),
                evidence: CapabilityEvidence::ProductionVerified,
                resume_evidence: ResumeEvidenceState::Confirmed,
                supported_actions: Vec::new(),
                action_matrix: ProviderActionMatrix::from_rows(rows).expect("rows"),
                trust: ProviderCapabilityEvidence::Unknown,
                probed_at: Some("2026-10-09T00:00:00Z".to_string()),
                probe_artifact_ref: Some("probe://boundary/codex/seed".to_string()),
                root_recipe_evidence: RootRecipeEvidence::None,
            },
        )
        .expect("seed");

    let state = WebAppState::with_events(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        EventHub::new(),
    )
    .with_capability_revalidate_service(fast_path_service(&root.path().join(".aria"), "1.42.0"));
    let app = build_web_router(state);
    std::mem::forget(root);

    let (status, body) = request_json(
        &app,
        Method::POST,
        &revalidate_uri(&lc_id),
        json!({ "provider_type": "codex" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["outcome"], json!("already_confirmed"), "body: {body}");
    assert_eq!(body["version"], json!("1.42.0"));
    // GET 投影同步显示 Confirmed（durable seed 可见）。
    let (_, snapshot) = request_json(&app, Method::GET, &capabilities_uri(&lc_id), json!({})).await;
    let codex = snapshot["providers"]
        .as_array()
        .expect("providers")
        .iter()
        .find(|provider| provider["provider_type"] == json!("codex"))
        .expect("codex");
    for row in codex["rows"].as_array().expect("rows") {
        assert_eq!(row["launch"], json!("confirmed"), "seed 行可见: {row}");
    }
}

#[tokio::test]
async fn capability_revalidate_probe_failure_reports_stable_error_and_keeps_durable() {
    let (app, paths, lc_id) = app_fixture(|root| Some(failing_service(&root.join(".aria"))));
    let (status, body) = request_json(
        &app,
        Method::POST,
        &revalidate_uri(&lc_id),
        json!({ "provider_type": "codex" }),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "body: {body}");
    assert_eq!(
        body["code"], "provider_capability_probe_failed",
        "body: {body}"
    );
    assert!(
        body["details"]["action"].is_string(),
        "失败点名 action: {body}"
    );
    assert!(
        body["details"]["detail"]
            .as_str()
            .expect("detail")
            .contains("boundary_unobserved"),
        "原始错误透传: {body}"
    );
    // durable 保持点击前状态（全 Unknown）。
    let (_, snapshot) = request_json(&app, Method::GET, &capabilities_uri(&lc_id), json!({})).await;
    let codex = snapshot["providers"]
        .as_array()
        .expect("providers")
        .iter()
        .find(|provider| provider["provider_type"] == json!("codex"))
        .expect("codex");
    for row in codex["rows"].as_array().expect("rows") {
        assert_ne!(row["launch"], json!("confirmed"), "失败不伪造: {row}");
    }
    let _ = paths;
}
