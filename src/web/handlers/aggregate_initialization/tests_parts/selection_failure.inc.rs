// -------------------------------------------------------------------
// 片4（REQ-BOOT-07 / REQ-PCR-02）：provider 选择期的 HTTP 失败面——
// 测试点 #4（全 Unknown）/ #5（health 降级）/ #6（无可用命令）/
// #7（选择成功的 trust 分流）与 8b（revalidate 六字段）/ 8c（Denied
// 的 fallback 位置规则）。既有「期望创建成功」的夹具走
// `with_selector_defaulting_to_claude`，不经本片段。
// -------------------------------------------------------------------

/// health 探针 stub：`missing` 里的可执行名报 CommandMissing，其余报
/// 成功（版本取各家最低兼容版本，与 providers.rs 的探测断言同源）。
struct SelectiveHealthRunner {
    missing: Vec<String>,
}

#[async_trait::async_trait]
impl crate::cross_cutting::bounded_command_runner::BoundedCommandRunner for SelectiveHealthRunner {
    async fn run(
        &self,
        request: crate::cross_cutting::bounded_command_runner::BoundedCommandRequest,
    ) -> Result<
        crate::cross_cutting::bounded_command_runner::BoundedCommandResult,
        crate::cross_cutting::bounded_command_runner::BoundedCommandError,
    > {
        if self.missing.iter().any(|name| name == &request.executable) {
            return Err(
                crate::cross_cutting::bounded_command_runner::BoundedCommandError::CommandMissing {
                    executable: request.executable.clone(),
                    details: "command not found".to_string(),
                },
            );
        }
        let version = match request.executable.as_str() {
            "codex" => "2.0",
            "pi" => "0.83.0",
            "kimi" => "0.34.0",
            _ => "1.0",
        };
        Ok(
            crate::cross_cutting::bounded_command_runner::BoundedCommandResult {
                exit_code: Some(0),
                stdout: version.to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                duration_ms: 1,
            },
        )
    }
}

/// 一枚 PlanningReadOnly launch=Denied 的能力行（其余与 Confirmed 种子
/// 同形）——构造「已安装但被真实拒绝」的候选，供 Denied 场景消费。
fn denied_capability_record(
    provider_type: crate::product::logical_codebase::provider_gateway::ProviderRefType,
    reason: &str,
) -> crate::product::logical_codebase::provider_capability_store::ProviderCapabilityRecord {
    use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
    use crate::product::logical_codebase::policy::SessionPolicyAction;
    use crate::product::logical_codebase::provider_capability_store::{
        ProviderActionCapability, ProviderActionMatrix,
    };
    let mut record = crate::web::handlers::automation_enrollment_test_support::confirmed_capability_record(provider_type);
    record.action_matrix = ProviderActionMatrix::from_rows(vec![ProviderActionCapability {
        action: SessionPolicyAction::PlanningReadOnly,
        launch: ProviderCapabilityEvidence::Denied {
            reason: reason.to_string(),
        },
        resume: ProviderCapabilityEvidence::Confirmed,
        write_boundary: ProviderCapabilityEvidence::Confirmed,
        projection_digest: "projection-digest-denied".to_string(),
        evidence_ref: "probe://denied".to_string(),
    }])
    .expect("denied probe rows");
    record
}

struct SelectionFailureFixture {
    app: axum::Router,
    lc_id: String,
    paths: ProductAppPaths,
}

/// 选择期失败面夹具：显式 LC（真实子树作用域，manifest/capability 都在
/// `logical-codebases/{lc_id}/` 下）+ 注入按场景定制的 health + 可选
/// trust 门（RouteTrustGate）。capability 先物化 bootstrap Unknown 行
/// （迁移桥退役后 material prep 的真实形态），各用例再按需种
/// Confirmed/Denied。
async fn selection_failure_fixture(
    missing: &[&str],
    trust_gate_fail: Option<bool>,
) -> SelectionFailureFixture {
    let root = tempdir().expect("root");
    let root_path = root.path().to_path_buf();
    let paths = ProductAppPaths::new(root_path.join(".aria"));
    ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "Selection failure wiring test".to_string(),
            description: None,
        })
        .unwrap();
    let aggregate_root = root_path.join("aggregate-root");
    std::fs::create_dir_all(&aggregate_root).unwrap();
    let lc_id = crate::product::logical_codebase::LogicalCodebaseStore::new(paths.clone())
        .create(
            "project_0001",
            crate::product::logical_codebase::LogicalCodebaseCreateInput {
                name: "selection-failure".to_string(),
                aggregate_root: aggregate_root.clone(),
            },
        )
        .unwrap()
        .id;
    crate::product::logical_codebase::LogicalCodebaseStore::for_lc(paths.clone(), lc_id.clone())
        .save_manifest(
            "project_0001",
            &crate::product::logical_codebase::store::LogicalCodebaseManifest::new(
                "project_0001",
                aggregate_root,
                Vec::new(),
            ),
        )
        .unwrap();
    // bootstrap 能力行（全 Unknown）：选择器 L2 的真实读取形态。
    crate::product::logical_codebase::ProviderCapabilityStore::new(paths.clone())
    .ensure_bootstrap()
    .expect("bootstrap capability rows");

    let factory = fake_registry_gateway_factory(paths.clone());
    let dependencies = match trust_gate_fail {
        Some(gate_fail) => {
            let gate = Arc::new(RouteTrustGate::new(gate_fail));
            let clock: Arc<dyn Fn() -> String + Send + Sync> =
                Arc::new(|| "2026-10-03T00:00:00Z".to_string());
            let coordinator = AggregateInitializationCoordinator::new(
                paths.clone(),
                AggregateInitializationOperationStore::new(paths.clone()),
                Arc::new(RouteTestSkills),
                Arc::new(DeterministicAggregatePreflightService::new(paths.clone())),
                Arc::new(GatewayFactoryProviderTurnDriver::new(
                    Some(factory.clone()),
                    Some(paths.clone()),
                )) as Arc<dyn AggregateProviderTurnDriver>,
                clock,
            )
            .with_trust(gate.clone());
            AggregateInitializationDependencies::new(
                Arc::new(coordinator),
                InitializationRunRegistry::default(),
            )
            .with_trust(gate)
        }
        None => build_test_dependencies(root_path.clone(), Some(factory)),
    };
    let runner: Arc<dyn crate::cross_cutting::bounded_command_runner::BoundedCommandRunner> =
        Arc::new(SelectiveHealthRunner {
            missing: missing.iter().map(|name| name.to_string()).collect(),
        });
    let health = Arc::new(
        crate::cross_cutting::provider_health::ProviderHealthService::with_dependencies(
            crate::cross_cutting::aria_state_paths::AriaStatePaths::from_workspace_root(&root_path),
            runner.clone(),
            Arc::new(crate::cross_cutting::provider_health::SystemProviderHealthClock),
            std::time::Duration::from_secs(1),
            4096,
        ),
    );
    let gate = Arc::new(
        crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate::new(
            health.clone(),
        ),
    );
    let state = WebAppState::new(root_path.clone(), WebRuntime::new_fake(root_path.clone()))
        .with_aggregate_initialization_dependencies(dependencies)
        .with_provider_health(health, gate, runner);
    std::mem::forget(root);
    SelectionFailureFixture {
        app: build_web_router(state),
        lc_id,
        paths,
    }
}

/// POST 创建入口（canonical v1.3 路由），返回 (status, body)。
async fn selection_failure_create(
    fixture: &SelectionFailureFixture,
    idempotency_key: &str,
) -> (StatusCode, serde_json::Value) {
    let uri = format!(
        "/api/projects/project_0001/logical-codebases/{}/initializations",
        fixture.lc_id
    );
    response_json(
        post_json(
            &fixture.app,
            &uri,
            serde_json::json!({"idempotency_key": idempotency_key}),
        )
        .await,
    )
    .await
}

/// 断言五步 operation 完全未被创建（选择期失败零写入）。
fn assert_no_operation_created(fixture: &SelectionFailureFixture) {
    let operations =
        AggregateInitializationOperationStore::for_lc(fixture.paths.clone(), &fixture.lc_id);
    assert!(
        operations
            .list("project_0001")
            .expect("list operations")
            .is_empty(),
        "selection failure must keep the five-step operation uncreated"
    );
}

/// 测试点 #4 + 8b：四家全 Unknown → `aggregate_initialization_provider_
/// needs_verification`，operation 未创建；`details.candidates` 四家齐全
/// （owner 顺序），`details.revalidate` 六字段逐个非空且只指向单家
/// （suggested=pi），`body.provider_type` 等于被点名 provider，`expect`
/// 说明真实探针会消耗额度（REQ-PCR-02 场景 2 的唯一落点）。
#[tokio::test]
async fn all_unverified_providers_reject_create_with_single_revalidate_entry() {
    let fixture = selection_failure_fixture(&[], None).await;
    let (status, body) = selection_failure_create(&fixture, "needs-verification-1").await;
    assert!(
        status.is_client_error() || status.is_server_error(),
        "needs verification must reject the create request: {status} {body}"
    );
    assert_eq!(
        body["code"], "aggregate_initialization_provider_needs_verification",
        "{body}"
    );
    assert_eq!(body["details"]["suggested_provider"], "pi", "{body}");
    assert_no_operation_created(&fixture);

    let candidates = body["details"]["candidates"]
        .as_array()
        .expect("candidate table");
    assert_eq!(candidates.len(), 4, "all four candidates listed: {body}");
    assert_eq!(candidates[0]["provider"], "pi");
    assert_eq!(candidates[3]["provider"], "codex");
    assert!(
        candidates
            .iter()
            .all(|candidate| !candidate["action_hint"]
                .as_str()
                .unwrap_or_default()
                .is_empty()),
        "every candidate carries an executable action hint: {body}"
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate["skipped_reason"] == "needs_verification"),
        "bootstrap rows are all unverified: {body}"
    );

    let revalidate = &body["details"]["revalidate"];
    for field in ["ui_path", "button_label", "method", "path", "body", "expect"] {
        assert!(
            !revalidate[field].to_string().is_empty(),
            "revalidate.{field} must be present and non-empty: {body}"
        );
    }
    assert_eq!(revalidate["method"], "POST", "{body}");
    // 能力面改机器级（REQ-PCR-08）：核验入口按机器级路由，不再按项目 / 逻辑
    // 代码库定位。
    assert_eq!(
        revalidate["path"],
        serde_json::json!("/api/providers/capabilities/revalidate"),
        "{body}"
    );
    assert_eq!(revalidate["body"]["provider_type"], "pi", "{body}");
    assert!(
        revalidate["expect"]
            .as_str()
            .expect("expect")
            .contains("额度"),
        "expect must disclose that the real probe spends quota: {body}"
    );
    assert!(
        !body["details"]["copy"]["title_zh"]
            .as_str()
            .unwrap_or_default()
            .is_empty(),
        "same-source failure copy must be attached: {body}"
    );
}

/// 测试点 #5：health 探测降级（快照持久化失败）→
/// `aggregate_initialization_provider_health_degraded`，operation 未创建。
#[tokio::test]
async fn degraded_provider_health_rejects_create_without_operation() {
    let mut fixture = selection_failure_fixture(&[], None).await;
    // 把快照落盘路径占成目录：下一次 refresh 的原子写必然失败 ⇒
    // `degraded=true` ⇒ 选择器返回 HealthDegraded。
    let health_file = fixture
        .paths
        .root()
        .join("state/provider-health.json");
    std::fs::create_dir_all(&health_file).expect("block the health snapshot file");
    let _ = &mut fixture;
    let (status, body) = selection_failure_create(&fixture, "health-degraded-1").await;
    assert!(
        status.is_client_error() || status.is_server_error(),
        "degraded health must reject the create request: {status} {body}"
    );
    assert_eq!(
        body["code"], "aggregate_initialization_provider_health_degraded",
        "{body}"
    );
    assert!(
        !body["details"]["detail"]
            .as_str()
            .unwrap_or_default()
            .is_empty(),
        "degraded detail must surface the internal reason: {body}"
    );
    assert_no_operation_created(&fixture);
}

/// 测试点 #6：四家命令都不可用 → `aggregate_initialization_no_provider_
/// available`，operation 未创建；逐家 `action_hint` 指向安装方式，
/// 不得归因为「未核验」。
#[tokio::test]
async fn no_available_cli_rejects_create_with_install_hints() {
    let fixture = selection_failure_fixture(&["claude", "codex", "pi", "kimi"], None).await;
    let (status, body) = selection_failure_create(&fixture, "no-cli-1").await;
    assert!(
        status.is_client_error() || status.is_server_error(),
        "no available cli must reject the create request: {status} {body}"
    );
    assert_eq!(
        body["code"], "aggregate_initialization_no_provider_available",
        "{body}"
    );
    assert_no_operation_created(&fixture);
    let candidates = body["details"]["candidates"]
        .as_array()
        .expect("candidate table");
    assert_eq!(candidates.len(), 4, "{body}");
    assert!(
        candidates.iter().all(|candidate| candidate["cli_available"] == false),
        "all four report their cli unavailable: {body}"
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate["action_hint"]
                .as_str()
                .unwrap_or_default()
                .contains("装好")),
        "every action hint must point at installing the cli: {body}"
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate["skipped_reason"] == "cli_unavailable"),
        "must not be attributed to needs_verification: {body}"
    );
    assert!(
        body["details"]["revalidate"].is_null(),
        "nothing installed ⇒ no revalidate entry: {body}"
    );
}

/// 测试点 #7a：选中 kimi_code 且 trust 未 Ready → 既有
/// `aggregate_initialization_trust_waiting` 链路（选择成功后才评估门）。
#[tokio::test]
async fn selected_kimi_without_trust_returns_existing_waiting_surface() {
    let fixture = selection_failure_fixture(&["pi"], Some(true)).await;
    // pi 未安装，kimi 已核验 ⇒ 选中 kimi_code ⇒ trust 门 Waiting。
    crate::web::handlers::automation_enrollment_test_support::seed_confirmed_capability_records(
        &fixture.paths,
        "project_0001",
        &[crate::product::logical_codebase::provider_gateway::ProviderRefType::KimiCode],
    );
    let (status, body) = selection_failure_create(&fixture, "kimi-trust-1").await;
    assert!(
        status.is_client_error() || status.is_server_error(),
        "trust waiting must reject the create request: {status} {body}"
    );
    assert_eq!(body["code"], "aggregate_initialization_trust_waiting", "{body}");
    assert_eq!(
        body["details"]["reason_code"],
        "kimi_trust_blocked_for_route_test",
        "{body}"
    );
    assert_no_operation_created(&fixture);
}

/// 测试点 #7b：选中 pi（requires_workspace_trust 过滤后为空）⇒ trust 门
/// no-op，直接开始五步；durable `input.provider == "pi"` 且候选表落账。
#[tokio::test]
async fn selected_pi_skips_trust_gate_and_freezes_provider() {
    let fixture = selection_failure_fixture(&["kimi", "claude", "codex"], Some(false)).await;
    crate::web::handlers::automation_enrollment_test_support::seed_confirmed_capability_records(
        &fixture.paths,
        "project_0001",
        &[crate::product::logical_codebase::provider_gateway::ProviderRefType::Pi],
    );
    let (status, body) = selection_failure_create(&fixture, "pi-direct-1").await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let operation_id = body["operation_id"]
        .as_str()
        .expect("operation id")
        .to_string();
    let operations =
        AggregateInitializationOperationStore::for_lc(fixture.paths.clone(), &fixture.lc_id);
    let operation = operations
        .get("project_0001", &operation_id)
        .expect("operation is durable");
    assert_eq!(operation.input.provider, "pi", "{body}");
    let selection = operation
        .provider_selection
        .expect("selection is recorded on the operation");
    assert_eq!(selection.selected, "pi");
    assert_eq!(selection.candidates.len(), 4);
}

/// 测试点 #8c：Denied 的 fallback 位置规则——被点名者（owner 顺序第一
/// 家已安装且被真实拒绝）的**下一家**是 `fallback_provider`，`revalidate`
/// 指向下一家；被点名者已是末家（codex）⇒ `fallback_provider == null`
/// 且不下发 `revalidate`（不指向不存在的下一家）。
#[tokio::test]
async fn denied_pi_points_fallback_at_next_owner_order_candidate() {
    let fixture = selection_failure_fixture(&["kimi", "claude", "codex"], None).await;
    let store = crate::product::logical_codebase::ProviderCapabilityStore::new(fixture.paths.clone());
    store
        .upsert(
            &denied_capability_record(
                crate::product::logical_codebase::provider_gateway::ProviderRefType::Pi,
                "planning probe rejected the read-only launch",
            ),
        )
        .expect("seed denied pi row");
    let (status, body) = selection_failure_create(&fixture, "denied-pi-1").await;
    assert!(
        status.is_client_error() || status.is_server_error(),
        "denied must reject the create request: {status} {body}"
    );
    assert_eq!(
        body["code"], "aggregate_initialization_provider_capability_denied",
        "{body}"
    );
    assert_no_operation_created(&fixture);
    assert_eq!(body["details"]["fallback_provider"], "kimi_code", "{body}");
    assert_eq!(
        body["details"]["revalidate"]["body"]["provider_type"],
        "kimi_code",
        "revalidate points at the next candidate: {body}"
    );
    let candidates = body["details"]["candidates"]
        .as_array()
        .expect("candidate table");
    assert_eq!(candidates.len(), 4, "{body}");
    assert_eq!(candidates[0]["capability_state"], "denied", "{body}");
    assert!(
        candidates
            .iter()
            .all(|candidate| !candidate["action_hint"]
                .as_str()
                .unwrap_or_default()
                .is_empty()),
        "every candidate carries an executable action hint: {body}"
    );
}

/// 测试点 #8c（末家）：codex 被点名时不存在下一家 ⇒ `fallback_provider`
/// 为 `null` 且 `revalidate` 不下发。
#[tokio::test]
async fn denied_codex_has_no_fallback_and_no_revalidate() {
    let fixture = selection_failure_fixture(&["pi", "kimi", "claude"], None).await;
    let store = crate::product::logical_codebase::ProviderCapabilityStore::new(fixture.paths.clone());
    store
        .upsert(
            &denied_capability_record(
                crate::product::logical_codebase::provider_gateway::ProviderRefType::Codex,
                "planning probe rejected the read-only launch",
            ),
        )
        .expect("seed denied codex row");
    let (status, body) = selection_failure_create(&fixture, "denied-codex-1").await;
    assert!(
        status.is_client_error() || status.is_server_error(),
        "denied must reject the create request: {status} {body}"
    );
    assert_eq!(
        body["code"], "aggregate_initialization_provider_capability_denied",
        "{body}"
    );
    assert!(body["details"]["fallback_provider"].is_null(), "{body}");
    assert!(
        body["details"]["revalidate"].is_null(),
        "must not point at a nonexistent next candidate: {body}"
    );
}
