// -------------------------------------------------------------------
// Task 1.8（REQ-REG-14/BOOT-03/D2）：生产 trust gate→recipe→receipt
// →readiness 接线。
// -------------------------------------------------------------------

/// 可切换 route 信任门 fake：`fail` 为真时返回稳定 reason_code 的可重试
/// Waiting；翻绿后同一调用面返回 Ready。
struct RouteTrustGate {
    fail: std::sync::atomic::AtomicBool,
    calls: std::sync::atomic::AtomicUsize,
}

impl RouteTrustGate {
    fn new(fail: bool) -> Self {
        Self {
            fail: std::sync::atomic::AtomicBool::new(fail),
            calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn set_ready(&self) {
        self.fail.store(false, std::sync::atomic::Ordering::SeqCst);
    }

    fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl crate::product::logical_codebase::ProviderTrustPrecondition for RouteTrustGate {
    fn ensure_before_recipe(
        &self,
        _project_id: &str,
        _operation_id: &str,
        _lc_id: &str,
        canonical_root: &std::path::Path,
        _providers: &[crate::product::models::ProviderName],
    ) -> crate::product::logical_codebase::ProviderTrustPreparationResult {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
            return crate::product::logical_codebase::ProviderTrustPreparationResult::Waiting {
                waiting: crate::product::logical_codebase::ProviderTrustWaiting {
                    provider: crate::product::models::ProviderName::KimiCode,
                    canonical_root: canonical_root.to_path_buf(),
                    trust_key: "wd_test_route_root_abc123def456".to_string(),
                    reason_code: "kimi_trust_blocked_for_route_test".to_string(),
                    message: "workspace trust entry is blocked for the route test".to_string(),
                    retry_action: "Resolve the blocked workspace trust entry, then retry \
                                       aggregate initialization."
                        .to_string(),
                    recorded_at: "2026-10-01T00:00:00Z".to_string(),
                },
            };
        }
        crate::product::logical_codebase::ProviderTrustPreparationResult::Ready {
            registrations: Vec::new(),
        }
    }
}

struct RouteTestSkills;

#[async_trait::async_trait]
impl AggregateSkillsPreparation for RouteTestSkills {
    async fn prepare_skills(
        &self,
        _project_id: &str,
        _operation_id: &str,
        _cancellation: CancellationToken,
    ) -> Result<MachineSkillsPreparation, AggregateInitializationError> {
        Ok(MachineSkillsPreparation {
            source_digest: "sha256:test-source".to_string(),
            link_digest: "sha256:test-links".to_string(),
            skills_root: PathBuf::from("/test-skills"),
            warnings: Vec::new(),
        })
    }
}
/// 首个 pre_check turn 注入一次中断的流式 provider（与 it_web
/// `FaultOncePreCheckStreamingProvider` 同构）：中断先于成功启动，audit
/// 不计 launch；后续 turn 委托 `FakeStreamingProvider`。
struct FaultOncePreCheckStreamingProvider {
    fired: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl crate::cross_cutting::streaming_provider::StreamingProviderAdapter
    for FaultOncePreCheckStreamingProvider
{
    async fn start(
        &self,
        input: crate::cross_cutting::streaming_provider::StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<
        crate::cross_cutting::streaming_provider::ProviderSession,
        crate::cross_cutting::provider_adapter::ProviderAdapterError,
    > {
        let pre_check =
            crate::product::repository_store::RepositoryInitializationStepKind::PreCheck
                .command()
                .expect("pre_check command");
        if input.prompt.contains(pre_check)
            && !self.fired.swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(
                crate::cross_cutting::provider_adapter::ProviderAdapterError::execution_failed(
                    None,
                    String::new(),
                    "test-injected interruption during the pre_check provider turn",
                    0,
                ),
            );
        }
        crate::cross_cutting::streaming_provider::FakeStreamingProvider
            .start(input, cancel)
            .await
    }
}

/// Task 1.8 fixture：与 `production()` 同形的依赖图——gateway factory 驱动
/// 携带 `Some(paths)`（admission 预检 + per-LC root recipe 命令审计），
/// coordinator 与 dependencies 双面注入可切换信任门。
struct TrustRouteFixture {
    app: axum::Router,
    lc_id: String,
    root: std::path::PathBuf,
    paths: ProductAppPaths,
    factory: Arc<LogicalCodebaseGatewayFactory>,
    gate: Arc<RouteTrustGate>,
    dependencies: AggregateInitializationDependencies,
}

fn trust_route_fixture(gate_fail: bool) -> TrustRouteFixture {
    let root = tempdir().expect("root");
    let root_path = root.path().to_path_buf();
    let paths = ProductAppPaths::new(root_path.join(".aria"));
    ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "Trust route wiring test".to_string(),
            description: None,
        })
        .unwrap();
    let aggregate_root = root_path.join("aggregate-root");
    std::fs::create_dir_all(&aggregate_root).unwrap();
    // 注意：此处不预置根规则入口 AGENTS.md——aggregate preflight 会拒绝
    // 已含 AGENTS.md 的聚合根（aggregate_root_ownership_conflict）；
    // 根规则材料在测试内的中断窗口写入（见各用例）。
    let lc_id = crate::product::logical_codebase::LogicalCodebaseStore::new(paths.clone())
        .create(
            "project_0001",
            crate::product::logical_codebase::LogicalCodebaseCreateInput {
                name: "trust-route".to_string(),
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

    // 与 it_web a03 同构：首个 pre_check turn 注入一次中断（audit 只记
    // 录成功启动，中断不计 launch），为「中断窗口写入根规则入口」留出
    // 无 worker 活跃的确定性时机。
    let factory = fake_registry_gateway_factory_with(
        Arc::new(FaultOncePreCheckStreamingProvider {
            fired: std::sync::atomic::AtomicBool::new(false),
        }),
        paths.clone(),
    );
    let gate = Arc::new(RouteTrustGate::new(gate_fail));
    let provider: Arc<dyn AggregateProviderTurnDriver> = Arc::new(
        GatewayFactoryProviderTurnDriver::new(Some(factory.clone()), Some(paths.clone())),
    );
    let clock: Arc<dyn Fn() -> String + Send + Sync> =
        Arc::new(|| "2026-10-01T00:00:00Z".to_string());
    let coordinator = AggregateInitializationCoordinator::new(
        paths.clone(),
        AggregateInitializationOperationStore::new(paths.clone()),
        Arc::new(RouteTestSkills),
        Arc::new(DeterministicAggregatePreflightService::new(paths.clone())),
        provider,
        clock,
    )
    .with_trust(gate.clone());
    let dependencies = AggregateInitializationDependencies::new(
        Arc::new(coordinator),
        InitializationRunRegistry::default(),
    )
    .with_trust(gate.clone());
    let state = WebAppState::new(root_path.clone(), WebRuntime::new_fake(root_path.clone()))
        .with_aggregate_initialization_dependencies(dependencies.clone());
    std::mem::forget(root);
    TrustRouteFixture {
        app: build_web_router(state),
        lc_id,
        root: root_path,
        paths,
        factory,
        gate,
        dependencies,
    }
}

#[tokio::test]
async fn production_root_initialization_trust_failure_keeps_recipe_unstarted() {
    let fixture = trust_route_fixture(true);
    let uri = format!(
        "/api/projects/project_0001/logical-codebases/{}/initializations",
        fixture.lc_id
    );
    let (status, body) = response_json(
        post_json(
            &fixture.app,
            &uri,
            serde_json::json!({"idempotency_key":"trust-fail-1"}),
        )
        .await,
    )
    .await;
    assert!(
        status.is_client_error() || status.is_server_error(),
        "trust waiting must reject the create request: {status} {body}"
    );
    // 等待面可重试：稳定 code + gate 上下文（provider/reason/retry action）。
    assert_eq!(
        body["code"], "aggregate_initialization_trust_waiting",
        "{body}"
    );
    assert_eq!(
        body["details"]["reason_code"],
        "kimi_trust_blocked_for_route_test"
    );
    assert!(
        !body["details"]["retry_action"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );

    // 五步 operation 不创建：durable store 为空。
    let operations =
        AggregateInitializationOperationStore::for_lc(fixture.paths.clone(), &fixture.lc_id);
    assert!(
        operations
            .list("project_0001")
            .expect("list operations")
            .is_empty(),
        "trust failure must keep the five-step operation uncreated"
    );
    // provider 启动计数为 0；gate 恰评估一次。
    assert_eq!(fixture.factory.audit().stream_launches(), 0);
    assert_eq!(fixture.gate.calls(), 1);
}

#[tokio::test]
async fn production_root_initialization_trust_retry_starts_claude_recipe() {
    let fixture = trust_route_fixture(true);
    let uri = format!(
        "/api/projects/project_0001/logical-codebases/{}/initializations",
        fixture.lc_id
    );
    let first = response_json(
        post_json(
            &fixture.app,
            &uri,
            serde_json::json!({"idempotency_key":"trust-retry-1"}),
        )
        .await,
    )
    .await;
    assert_eq!(
        first.1["code"], "aggregate_initialization_trust_waiting",
        "first attempt must surface the retryable waiting surface"
    );
    assert_eq!(fixture.factory.audit().stream_launches(), 0);

    // 等待面可重试：gate 翻绿后同一幂等键重放，五步 recipe 启动。
    fixture.gate.set_ready();
    let (status, body) = response_json(
        post_json(
            &fixture.app,
            &uri,
            serde_json::json!({"idempotency_key":"trust-retry-1"}),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let operation_id = body["operation_id"]
        .as_str()
        .expect("operation id")
        .to_string();

    let operations =
        AggregateInitializationOperationStore::for_lc(fixture.paths.clone(), fixture.lc_id.clone());
    let poll_operation = |operation_id: String| {
        let operations = operations.clone();
        async move {
            for _ in 0..300 {
                let operation = operations
                    .get("project_0001", &operation_id)
                    .expect("operation is durable");
                match operation.status {
                        crate::product::logical_codebase::AggregateInitializationOperationStatus::Created
                        | crate::product::logical_codebase::AggregateInitializationOperationStatus::Running => {
                            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                        }
                        _ => return operation,
                    }
            }
            panic!("operation {operation_id} did not reach a terminal state");
        }
    };
    // 首个 pre_check turn 注入一次中断（fixture 同 it_web a03）：provider
    // 启动计数仍为 0（中断先于成功启动）。
    let interrupted = poll_operation(operation_id.clone()).await;
    assert_eq!(
        interrupted.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Failed,
        "injected pre_check interruption must fail the operation: {:?}",
        interrupted.error
    );
    assert_eq!(fixture.factory.audit().stream_launches(), 0);

    // 中断窗口（无 worker 活跃）写入根规则入口：真实链路由 provider
    // recipe 生成根规则材料；fake provider 场景在此窗口落盘，重试的
    // 命令审计窗口将其作为既有材料冻结（receipt 与 readiness 投影共用
    // root_rule_digest 冻结其 digest）。
    std::fs::write(
        fixture.root.join("aggregate-root").join("AGENTS.md"),
        "# aggregate root rules\n",
    )
    .unwrap();

    // 显式 Retry（member_index continue 面）从 checkpoint 续跑。
    let action_uri = format!(
        "/api/projects/project_0001/logical-codebases/{}/bootstrap/actions",
        fixture.lc_id
    );
    let (status, action) = response_json(
        post_json(
            &fixture.app,
            &action_uri,
            serde_json::json!({
                "command_id": "cmd-trust-route-member-index-retry-1",
                "step": "member_index",
                "action": "retry",
                "expected_revision": null,
                "expected_object_id": operation_id,
            }),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{action}");
    assert_eq!(action["outcome"], "accepted");
    let completed = poll_operation(operation_id.clone()).await;
    assert_eq!(
        completed.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Completed,
        "retry after trust readiness must finish the five-step recipe: {:?}",
        completed.error
    );

    // 命令与 receipt 可查询：四条命令 receipt 全部 Allowed。
    let receipts = crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(
        fixture.paths.clone(),
        fixture.lc_id.clone(),
    );
    let commands = receipts
        .list_commands("project_0001", &operation_id)
        .expect("command receipts");
    assert_eq!(commands.len(), 4, "one audit receipt per frozen command");
    assert!(commands.iter().all(|receipt| {
            receipt.verdict
                == crate::product::logical_codebase::root_recipe_receipt::RootRecipeCommandVerdict::Allowed
        }));

    // 最终 receipt 冻结 canonical root/policy/rule 三重身份（rule digest
    // 经与 readiness 投影共享的 root_rule_digest 计算）。
    let receipt = receipts
        .get("project_0001", &operation_id)
        .expect("final receipt read")
        .expect("finalized root receipt");
    assert_eq!(
        receipt.canonical_root,
        std::fs::canonicalize(fixture.root.join("aggregate-root")).unwrap()
    );
    let rule_digest = crate::product::logical_codebase::root_recipe_receipt::root_rule_digest(
        &receipt.canonical_root,
    )
    .expect("root rule digest read")
    .expect("seeded root rule entry");
    assert_eq!(receipt.rule_digest, rule_digest);
    let policy_digest =
        crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::for_lc(
            fixture.paths.clone(),
            fixture.lc_id.clone(),
        )
        .get("project_0001")
        .expect("policy artifact read")
        .expect("bootstrap policy artifact")
        .digest;
    assert_eq!(receipt.policy_digest, policy_digest);

    // 三个 provider turn 经 gateway factory 启动；gate 第二次评估即 Ready。
    assert_eq!(fixture.factory.audit().stream_launches(), 3);
    assert_eq!(fixture.gate.calls(), 2);
}

#[tokio::test]
async fn aggregate_initialization_get_does_not_start_provider() {
    let fixture = trust_route_fixture(false);
    create_running_initialization(
        &fixture.paths,
        &fixture.lc_id,
        "aggregate_initialization_get_probe_0001",
    );
    let uri = format!(
        "/api/projects/project_0001/logical-codebases/{}/initializations/\
             aggregate_initialization_get_probe_0001",
        fixture.lc_id
    );
    // 重复 GET 是纯投影：operation 原样、provider 零启动、trust 门不评估。
    for _ in 0..3 {
        let (status, body) = get_json(&fixture.app, &uri).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body["status"], "running",
            "GET must not recover or advance the operation: {body}"
        );
    }
    assert_eq!(fixture.factory.audit().stream_launches(), 0);
    assert_eq!(
        fixture.gate.calls(),
        0,
        "GET must not evaluate the trust gate"
    );
}
