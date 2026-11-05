//! add-provider-revalidate-probe（Task 2）：capability 重验证 HTTP 面的
//! it_web 验收。
//!
//! 能力面已改**机器级**（REQ-PCR-08）：路由不再按项目 / 逻辑代码库定位 ⇒
//! 本文件走 `/api/providers/capabilities*`；旧的按项目 / 逻辑代码库定位的
//! 路由整条删除（404 断言见
//! `tests/it_web/web_provider_capabilities_machine_level.rs`）。
//!
//! - GET `/api/providers/capabilities`：只读投影四家 durable
//!   capability 行（未探测全 Unknown；bootstrap 默认记录不显示 Confirmed）；
//!   GET 不触发任何探针/写入。
//! - POST `/api/providers/capabilities/revalidate`：
//!   - Fake/未知 provider_type → 422 `provider_capability_probe_unsupported`，零探针、零 durable 写入；
//!   - 全行已 Confirmed@当前版本（注入版本源）→ 200 `already_confirmed`，秒回（探针 runner 被调用即失败——证明不重跑）；
//!   - 探针失败（注入恒败 runner）→ 稳定码 `provider_capability_probe_failed`，携带 action/detail，durable 保持点击前状态。
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

/// 机器级能力面路由（provider 是平台公用能力，与项目 / 逻辑代码库无关）。
fn capabilities_uri() -> String {
    "/api/providers/capabilities".to_string()
}

fn revalidate_uri() -> String {
    "/api/providers/capabilities/revalidate".to_string()
}

/// 旧的按项目 / 逻辑代码库定位的路由（REQ-PCR-08 整条删除 ⇒ 必须 404）。
fn legacy_revalidate_uri(lc_id: &str) -> String {
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
    let (app, _paths, _lc_id) = app_fixture(|_| None);
    let (status, body) = request_json(&app, Method::GET, &capabilities_uri(), json!({})).await;
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
    let (_, body_again) = request_json(&app, Method::GET, &capabilities_uri(), json!({})).await;
    assert_eq!(body, body_again, "GET 必须零副作用");
}

#[tokio::test]
async fn capability_revalidate_rejects_fake_provider_without_probe_or_write() {
    let (app, paths, _lc_id) = app_fixture(|_| None);
    let (status, body) = request_json(
        &app,
        Method::POST,
        &revalidate_uri(),
        json!({ "provider_type": "fake" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body: {body}");
    assert_eq!(body["code"], "provider_capability_probe_unsupported");
    // 零 durable 写入：capabilities.json 不因拒绝产生。
    // capability 记录作用域 = 机器级（REQ-PCR-07）。
    let capabilities = paths.provider_capabilities_root().join("capabilities.json");
    assert!(
        !capabilities.exists(),
        "拒绝路径不得写 capability 记录: {}",
        capabilities.display()
    );
}

#[tokio::test]
async fn capability_revalidate_unknown_provider_type_is_unsupported() {
    let (app, _paths, _lc_id) = app_fixture(|_| None);
    let (status, body) = request_json(
        &app,
        Method::POST,
        &revalidate_uri(),
        json!({ "provider_type": "not-a-provider" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body: {body}");
    assert_eq!(body["code"], "provider_capability_probe_unsupported");
}

/// REQ-PCR-08：旧的按项目 / 逻辑代码库定位的核验路由**整条删除** ⇒ 请求旧
/// 路由 404，且不是转发到机器级路由（能力面不再按项目 / 逻辑代码库定位，
/// 故「未知 LC ⇒ 404」这一旧守卫随路由一并退场）。
#[tokio::test]
async fn capability_revalidate_legacy_project_scoped_route_is_404() {
    let (app, _paths, lc_id) = app_fixture(|_| None);
    let (status, body) = request_json(
        &app,
        Method::POST,
        &legacy_revalidate_uri(&lc_id),
        json!({ "provider_type": "codex" }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(
        body.get("providers").is_none(),
        "旧路由不得转发到机器级能力面: {body}"
    );
}

/// fast-path：seed 全行 Confirmed@1.42.0（durable 直建，全 pub 材料包），
/// 注入同版本版本源 → POST 秒回 already_confirmed，探针 runner 被触发即
/// panic（`fast_path_service`）。
#[tokio::test]
async fn capability_revalidate_confirmed_rows_fast_return_without_probe() {
    // seed 由产品层 store 完成（pub API）。
    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let _lc_id = create_project_and_lc(&paths, &root.path().join("aggregate-root"));
    use cadence_aria::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
    use cadence_aria::product::logical_codebase::policy::{
        ProviderDialect, ProviderWireDialect, SessionPolicyAction,
    };
    use cadence_aria::product::logical_codebase::provider_capability_store::{
        CapabilityEvidence, PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION, ProviderActionCapability,
        ProviderActionMatrix, ProviderCapabilityRecord, ProviderCapabilityStore,
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
    ProviderCapabilityStore::new(paths.clone())
        .upsert(
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
                execution_semantics:
                    cadence_aria::cross_cutting::provider_boundary::PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN
                        .to_string(),
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
        &revalidate_uri(),
        json!({ "provider_type": "codex" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["outcome"], json!("already_confirmed"), "body: {body}");
    assert_eq!(body["version"], json!("1.42.0"));
    // GET 投影同步显示 Confirmed（durable seed 可见）。
    let (_, snapshot) = request_json(&app, Method::GET, &capabilities_uri(), json!({})).await;
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
    let (app, paths, _lc_id) = app_fixture(|root| Some(failing_service(&root.join(".aria"))));
    let (status, body) = request_json(
        &app,
        Method::POST,
        &revalidate_uri(),
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
    let (_, snapshot) = request_json(&app, Method::GET, &capabilities_uri(), json!({})).await;
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

/// 批2 Task 9:旧 v2 Confirmed 记录(生产 store 路径、真实文件字节)读取
/// 时当前行必须 Unknown——只有真实重探针(run_cli_boundary_probe 真实
/// 执行,CLI 以脚本替身)导入 host-open 证据后才可见;证据文件确实存在,
/// durable 升为 schema 3 + host-open-v1。不能只测 mock:本测试走生产
/// store 路径与真实 probe→2c/2d 导入链。
#[tokio::test]
async fn capability_revalidate_old_v2_confirmed_requires_real_reprobe() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let _lc_id = create_project_and_lc(&paths, &root.path().join("aggregate-root"));

    // 旧 v2 durable 字节(三格 Confirmed @ 当前版本)直接落在生产 store
    // 路径——历史记录,不得被读取改写。
    let fake_version = "aria-fake-cli 1.42.0";
    let confirmed_row = r#"{
        "action": "{action}",
        "launch": {"state": "confirmed"},
        "resume": {"state": "confirmed"},
        "write_boundary": {"state": "confirmed"},
        "projection_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        "evidence_ref": "probe://boundary/old-v2"
      }"#;
    let old_v2_json = format!(
        r#"[{{
  "provider_type": "codex",
  "schema_version": 2,
  "version": "{fake_version}",
  "adapter_dialect": "codex_cli_v1",
  "wire_dialect": "codex-app-server-rpc",
  "capability_snapshot_ref": "cap_managed_snapshot",
  "evidence": "production_verified",
  "resume_evidence": "confirmed",
  "supported_actions": [],
  "action_matrix": [
    {},
    {},
    {}
  ],
  "trust": {{"state": "confirmed"}},
  "probed_at": "2026-10-01T00:00:00Z",
  "probe_artifact_ref": "probe://boundary/old-v2",
  "root_recipe_evidence": {{"kind": "none"}}
}}]"#,
        confirmed_row.replace("{action}", "coding_target_write"),
        confirmed_row.replace("{action}", "planning_read_only"),
        confirmed_row.replace("{action}", "review_read_only"),
    );
    // capability 记录作用域 = 机器级（REQ-PCR-07）：旧 v2 字节播种在机器级
    // 路径上（迁移面按同一路径读）。
    let capabilities_path = paths.provider_capabilities_root().join("capabilities.json");
    std::fs::create_dir_all(capabilities_path.parent().expect("dir")).expect("create dir");
    std::fs::write(&capabilities_path, &old_v2_json).expect("seed old v2 bytes");

    // 真实探针机 CLI 替身:仅 `--version`(resume 面由规格覆盖;替身
    // 无法应答 resume 通道,resume 面保持 Unknown,如实导入)。
    use std::os::unix::fs::PermissionsExt;
    let cli_path = root.path().join("fake-codex-cli");
    std::fs::write(
        &cli_path,
        format!("#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo '{fake_version}'; exit 0; fi\nexit 1\n"),
    )
    .expect("write fake cli");
    std::fs::set_permissions(&cli_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod fake cli");
    let cli_text = cli_path.to_string_lossy().into_owned();

    let probe_calls = Arc::new(AtomicUsize::new(0));
    let calls_for_runner = probe_calls.clone();
    // runner = 真实 run_cli_boundary_probe(生产链),仅 CLI 程序指向替身、
    // resume 面关闭(替身无应答通道)。
    let service = Arc::new(
        ProviderCapabilityRevalidateService::new(ProductAppPaths::new(root.path().join(".aria")))
            .with_version_source(Arc::new(move |_program: &str| Ok(fake_version.to_string())))
            .with_probe_runner(Arc::new(move |invocation: ProbeInvocation| {
                let cli_text = cli_text.clone();
                let calls = calls_for_runner.clone();
                Box::pin(async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    cadence_aria::product::logical_codebase::provider_boundary_probe::
                    run_cli_boundary_probe(
                        invocation.provider,
                        &cli_text,
                        invocation.action,
                        &invocation.base,
                        &invocation.evidence_root,
                        &invocation.session_label,
                        None,
                        invocation.scope,
                    )
                    .await
                })
            })),
    );
    let state = WebAppState::with_events(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        EventHub::new(),
    )
    .with_capability_revalidate_service(service);
    let app = build_web_router(state);
    std::mem::forget(root);

    // 1) GET:旧 v2 Confirmed 不可见(当前行 Unknown),且 GET 不改写历史字节。
    let (status, snapshot) = request_json(&app, Method::GET, &capabilities_uri(), json!({})).await;
    assert_eq!(status, StatusCode::OK, "body: {snapshot}");
    let codex = snapshot["providers"]
        .as_array()
        .expect("providers")
        .iter()
        .find(|provider| provider["provider_type"] == json!("codex"))
        .expect("codex");
    for row in codex["rows"].as_array().expect("rows") {
        assert_eq!(
            row["launch"],
            json!("unknown"),
            "旧 v2 Confirmed 不得成为当前 Confirmed: {row}"
        );
        assert_eq!(
            row["write_boundary"],
            json!("unknown"),
            "旧 v2 Confirmed 不得成为当前 Confirmed: {row}"
        );
    }
    assert_eq!(
        std::fs::read(&capabilities_path).expect("reread"),
        old_v2_json.as_bytes(),
        "GET 读取不得改写历史文件字节"
    );

    // 2) POST revalidate:真实重探针 + 2c/2d 导入(生产 store 路径)。
    let (status, body) = request_json(
        &app,
        Method::POST,
        &revalidate_uri(),
        json!({ "provider_type": "codex" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["outcome"], json!("revalidated"), "body: {body}");
    assert_eq!(body["version"], json!(fake_version));
    assert_eq!(
        probe_calls.load(Ordering::SeqCst),
        3,
        "三 action 各一次真实探针"
    );

    // durable 升级为 schema 3 + host-open-v1;证据文件确实存在。
    let durable = std::fs::read_to_string(&capabilities_path).expect("read durable");
    assert!(
        durable.contains("\"schema_version\": 3"),
        "durable: {durable}"
    );
    assert!(durable.contains("host-open-v1"), "durable: {durable}");

    // 3) GET:重探针后的行可见(launch/write Confirmed;resume 替身
    //    无应答通道,如实 Unknown——fresh/resume 分格)。
    let (_, snapshot) = request_json(&app, Method::GET, &capabilities_uri(), json!({})).await;
    let codex = snapshot["providers"]
        .as_array()
        .expect("providers")
        .iter()
        .find(|provider| provider["provider_type"] == json!("codex"))
        .expect("codex");
    let mut evidence_refs = Vec::new();
    for row in codex["rows"].as_array().expect("rows") {
        assert_eq!(row["launch"], json!("confirmed"), "重探针后可见: {row}");
        assert_eq!(
            row["write_boundary"],
            json!("confirmed"),
            "重探针后可见: {row}"
        );
        assert_eq!(row["resume"], json!("unknown"), "resume 分格如实: {row}");
        evidence_refs.push(
            row["evidence_ref"]
                .as_str()
                .expect("evidence_ref")
                .to_string(),
        );
    }
    for evidence_ref in evidence_refs {
        assert!(
            Path::new(&evidence_ref).is_file(),
            "证据文件必须真实存在: {evidence_ref}"
        );
    }

    // 4) 再次 POST:resume 未 Confirmed → 不秒回,重新真实探针(逐格
    //    如实,不因 launch/write Confirmed 而冒充全行)。
    let (status, body) = request_json(
        &app,
        Method::POST,
        &revalidate_uri(),
        json!({ "provider_type": "codex" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(
        body["outcome"],
        json!("revalidated"),
        "resume 未确认不秒回: {body}"
    );
    assert_eq!(
        probe_calls.load(Ordering::SeqCst),
        6,
        "第二次点击重新逐 action 真实探针"
    );
}
