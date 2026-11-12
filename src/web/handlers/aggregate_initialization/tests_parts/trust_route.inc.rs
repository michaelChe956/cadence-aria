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
    fn ensure_ready(
        &self,
        _project_id: &str,
        _operation_id: &str,
        _lc_id: &str,
        canonical_root: &std::path::Path,
        _providers: &[crate::product::models::ProviderName],
        _scenario: crate::product::logical_codebase::provider_trust::ProviderTrustScenario,
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
                    manual_guidance: "Resolve the workspace-trust entry for this root, \
                                       then retry aggregate initialization."
                        .to_string(),
                    command_markdown: None,
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
/// Task 4：测试 recipe 的根规则材料（provider 在 RuleAndMcpConfig 命令
/// 时机落盘，与真实 Claude 生成根规则同构）。
const ROOT_POLICY_TEST_AGENTS: &str =
    "# aggregate root rules\n\n- 根入口规则：成员仓统一遵守语言与工程规范。\n";
const ROOT_POLICY_TEST_LANGUAGE: &str = "# 语言规则\n\n- 必须使用中文回答。\n";
/// `/project-rules-examples` 的真实落盘面（`cadence/project-rules/examples`
/// ——见 ROOT_RECIPE_ALLOWLIST 的实测证据）：零动作 turn 会被判失败，故测试
/// provider 必须按真实命令落盘面产生文件变更。
const ROOT_POLICY_TEST_EXAMPLES: &str = "# 根规则示例\n\n- 示例：成员仓统一使用中文注释。\n";

/// `trust_route_fixture` 的可调选项（Task 4）。
#[derive(Debug, Clone)]
struct RecipeFixtureOptions {
    /// 首个 pre_check turn 注入一次中断（保留既有 trust retry 弧的确定性
    /// 无 worker 观察窗口；中断先于成功启动，audit 不计 launch）。
    fault_first_pre_check: bool,
    /// 首个末命令（openspec_and_examples）turn 注入一次中断（末命令审计
    /// 前的确定性观察窗口）。
    fault_first_final_turn: bool,
    /// RuleAndMcpConfig turn 是否落盘 `.claude/rules/language.md`
    /// （缺 language 的发布负例置 false）。
    write_language_rule: bool,
    /// REQ-BOOT-17 负例：pre_check turn **零动作**（不产生任何落盘）——
    /// 该 step 必须判失败（零动作 turn 不得静默通过）。
    zero_change_pre_check: bool,
    /// pre_check turn 的**脚本报告**（工具结果原文）。`Some(report)` 时 turn
    /// 产出该报告且零落盘——「命令确实执行且按契约幂等无变更」的形态。
    recipe_report: Option<String>,
}

impl Default for RecipeFixtureOptions {
    fn default() -> Self {
        Self {
            fault_first_pre_check: false,
            fault_first_final_turn: false,
            write_language_rule: true,
            zero_change_pre_check: false,
            recipe_report: None,
        }
    }
}

/// 产出「一条工具结果 + 完成」的 provider session（脚本报告形态）：
/// 工具结果原文即脚本 stdout，平台据此读出报告结论。
fn reported_provider_session(report: &str) -> crate::cross_cutting::streaming_provider::ProviderSession {
    use crate::cross_cutting::streaming_provider::{
        ProviderCompletion, ProviderEvent, ProviderToolResult,
    };
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(8);
    let (command_tx, _command_rx) = tokio::sync::mpsc::channel(4);
    let report = report.to_string();
    tokio::spawn(async move {
        let _ = event_tx
            .send(ProviderEvent::ToolResult(ProviderToolResult {
                tool_use_id: "script_report".to_string(),
                output: report,
                is_error: false,
            }))
            .await;
        let _ = event_tx
            .send(ProviderEvent::Completed(ProviderCompletion::plain(
                "recipe turn complete",
                None,
            )))
            .await;
    });
    crate::cross_cutting::streaming_provider::ProviderSession {
        events: event_rx,
        commands: command_tx,
        native_session_id: None,
    }
}

/// Task 4 生产同构 provider：在真正执行 RuleAndMcpConfig 命令的时机向
/// 聚合根落盘 AGENTS/language 规则材料（替代旧的「预写 AGENTS 后 fake
/// 成功」），并可按选项在首个 pre_check / 末命令 turn 注入一次中断。
/// 该 seam 只存在于测试依赖图，绝不扩大到真实 E2E。
struct RootPolicyRecipeStreamingProvider {
    options: RecipeFixtureOptions,
    pre_check_fault_fired: std::sync::atomic::AtomicBool,
    final_turn_fault_fired: std::sync::atomic::AtomicBool,
}

impl RootPolicyRecipeStreamingProvider {
    fn new(options: RecipeFixtureOptions) -> Self {
        Self {
            options,
            pre_check_fault_fired: std::sync::atomic::AtomicBool::new(false),
            final_turn_fault_fired: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn interruption(message: &str) -> crate::cross_cutting::provider_adapter::ProviderAdapterError {
        crate::cross_cutting::provider_adapter::ProviderAdapterError::execution_failed(
            None,
            String::new(),
            message.to_string(),
            0,
        )
    }
}

#[async_trait::async_trait]
impl crate::cross_cutting::streaming_provider::StreamingProviderAdapter
    for RootPolicyRecipeStreamingProvider
{
    async fn start(
        &self,
        input: crate::cross_cutting::streaming_provider::StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<
        crate::cross_cutting::streaming_provider::ProviderSession,
        crate::cross_cutting::provider_adapter::ProviderAdapterError,
    > {
        use crate::product::repository_store::RepositoryInitializationStepKind as RepoStep;
        let pre_check = RepoStep::PreCheck.command().expect("pre_check command");
        let rule_config = RepoStep::RuleConfig.command().expect("rule-config command");
        let project_rules_examples = RepoStep::ProjectRulesExamples
            .command()
            .expect("project-rules-examples command");
        if self.options.fault_first_pre_check
            && input.prompt.contains(pre_check)
            && !self
                .pre_check_fault_fired
                .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(Self::interruption(
                "test-injected interruption during the pre_check provider turn",
            ));
        }
        if self.options.fault_first_final_turn
            && input.prompt.contains(project_rules_examples)
            && !self
                .final_turn_fault_fired
                .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(Self::interruption(
                "test-injected interruption during the final command provider turn",
            ));
        }
        // RuleAndMcpConfig 命令时机：真实 provider 生成根规则的位置——
        // 测试 provider 在 working_dir（聚合根）落盘 AGENTS 入口与
        // language 规则，使末命令发布收口有真实材料可聚合。
        if input.prompt.contains(rule_config) {
            let root = input.working_dir.clone();
            std::fs::write(root.join("AGENTS.md"), ROOT_POLICY_TEST_AGENTS)
                .expect("write root AGENTS.md");
            if self.options.write_language_rule {
                std::fs::create_dir_all(root.join(".claude").join("rules"))
                    .expect("create root rules dir");
                std::fs::write(
                    root.join(".claude").join("rules").join("language.md"),
                    ROOT_POLICY_TEST_LANGUAGE,
                )
                .expect("write root language rule");
            }
        }
        // REQ-BOOT-17：零动作的 provider turn 会被判失败（该 step 失败），
        // 故测试 provider 按真实四命令的落盘面同构——每个 provider turn 都
        // 必须产生真实文件变更：
        // - pre_check 写 `.claude/`（真实 pre-check 写 .agents/.claude/...）；
        // - project_rules_examples 写 `cadence/project-rules/examples`
        //   （真实落盘面，见 ROOT_RECIPE_ALLOWLIST 的实测证据）。
        // 「命令确实执行且按契约幂等无变更」形态：turn 产出脚本报告（工具
        // 结果原文）但不产生任何落盘——零变更判据据此判成功。
        if input.prompt.contains(pre_check) && self.options.recipe_report.is_some() {
            let report = self
                .options
                .recipe_report
                .clone()
                .expect("recipe report option present");
            return Ok(reported_provider_session(&report));
        }
        if input.prompt.contains(pre_check) && !self.options.zero_change_pre_check {
            let claude_dir = input.working_dir.join(".claude");
            std::fs::create_dir_all(&claude_dir).expect("create root .claude dir");
            // 追加式落盘：每次 turn 都有真实体积变化（预初始化根与重试同样
            // 可观测——零动作 turn 会被判失败，夹具不得产出「零变更」turn）。
            let mut marker = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(claude_dir.join("settings.json"))
                .expect("open root claude settings");
            std::io::Write::write_all(&mut marker, b"{\"pre_check\":true}\n")
                .expect("append root claude settings");
        }
        if input.prompt.contains(project_rules_examples) {
            let examples = input
                .working_dir
                .join("cadence")
                .join("project-rules")
                .join("examples");
            std::fs::create_dir_all(&examples).expect("create project rules examples dir");
            std::fs::write(
                examples.join("examples.md"),
                ROOT_POLICY_TEST_EXAMPLES,
            )
            .expect("write project rules examples");
        }
        crate::cross_cutting::streaming_provider::FakeStreamingProvider
            .start(input, cancel)
            .await
    }
    /// Task 1b 段③:prepared launch 经 `start_validated` 分发——拆出 input
    /// 后与裸 `start` 同路径(recipe 命令时机行为保持)。
    async fn start_validated(
        &self,
        validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<
        crate::cross_cutting::streaming_provider::ProviderSession,
        crate::cross_cutting::provider_adapter::ProviderAdapterError,
    > {
        let (input, _launch) = validated.into_parts();
        self.start(input, cancel).await
    }
}

/// Task 1.8 fixture：与 `production()` 同形的依赖图——gateway factory 驱动
/// 携带 `Some(paths)`（admission 预检 + per-LC root recipe 命令审计 +
/// Task 4 末命令发布收口），coordinator 与 dependencies 双面注入可切换
/// 信任门。Task 4 起根规则材料由 provider 在 RuleAndMcpConfig 命令时机
/// 生成（生产同构），不再手工预写 AGENTS。
struct TrustRouteFixture {
    app: axum::Router,
    lc_id: String,
    root: std::path::PathBuf,
    paths: ProductAppPaths,
    factory: Arc<LogicalCodebaseGatewayFactory>,
    gate: Arc<RouteTrustGate>,
    /// 生产形依赖图；当前无读取点，保留 fixture 形状。
    #[allow(dead_code)]
    dependencies: AggregateInitializationDependencies,
}

fn trust_route_fixture(gate_fail: bool) -> TrustRouteFixture {
    trust_route_fixture_with(gate_fail, RecipeFixtureOptions::default())
}

/// Task 4：`trust_route_fixture` 的参数化版本——规则材料与一次性中断
/// 按 [`RecipeFixtureOptions`] 注入；claude_code 种为 Confirmed（选择器
/// 前置与 provider turn 用例同源）。
fn trust_route_fixture_with(gate_fail: bool, options: RecipeFixtureOptions) -> TrustRouteFixture {
    trust_route_fixture_full(gate_fail, options, true)
}

/// owner 2026-11-04 现场同构：L1 health 全可用但**不种任何已核验能力**——
/// provider 选择门（L2）以 `aggregate_initialization_provider_needs_
/// verification` 拦下根配方创建链（四家全「未探测」的真机截图状态）。
fn trust_route_fixture_unverified_providers() -> TrustRouteFixture {
    trust_route_fixture_full(false, RecipeFixtureOptions::default(), false)
}

/// `trust_route_fixture_with` 的完整内核：`seed_confirmed_provider = false`
/// 时不种 Confirmed capability（保持 provider 选择门可命中）。
fn trust_route_fixture_full(
    gate_fail: bool,
    options: RecipeFixtureOptions,
    seed_confirmed_provider: bool,
) -> TrustRouteFixture {
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
    // 说明：本夹具不预置根规则入口 AGENTS.md —— 聚合初始化的四命令由测试
    // provider 在 RuleAndMcpConfig 时机生成该材料。登记预检**不再**以
    // AGENTS.md / CLAUDE.md 的存在为门禁（2026-10-29 起），此处只是保持"未初始化根"语义。
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

    let factory = fake_registry_gateway_factory_with(
        Arc::new(RootPolicyRecipeStreamingProvider::new(options)),
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
    .with_trust(gate.clone())
    // owner 2026-10-29：与生产依赖图同形——聚合根已做过 Cadence-skills
    // 初始化时跳过四条命令，只发布聚合政策并签发跳过式 receipt。
    .with_policy_publication(Arc::new(SkippedAggregatePolicyPublisher {
        paths: paths.clone(),
    }));
    // Task 4：index op 指向本 fixture 的 LC scope，并以必缺的 codegraph
    // 二进制令 detached build 确定性失败——不与并行测试共享 temp 根，
    // 也不依赖宿主是否安装 codegraph（recipe 完成后的索引门独立性可
    // 确定性断言，见 root_policy_recipe_completed_keeps_index_gate_independent）。
    let index = Arc::new(AggregateIndexOperation::new(
        paths.clone(),
        CodeGraphCli::new(
            Arc::new(TokioBoundedCommandRunner),
            "codegraph-missing-binary-for-root-policy-tests".to_string(),
        ),
        CodeGraphExcludeGenerator,
    ));
    let dependencies = AggregateInitializationDependencies::with_index(
        Arc::new(coordinator),
        InitializationRunRegistry::default(),
        index,
    )
    .with_trust(gate.clone());
    let raw_state = WebAppState::new(root_path.clone(), WebRuntime::new_fake(root_path.clone()));
    let state = if seed_confirmed_provider {
        with_selector_defaulting_to_claude(raw_state, &root_path, &paths, )
    } else {
        with_provider_health_all_available(raw_state, &root_path)
    }
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
    assert!(
        !body["details"]["manual_guidance"]
            .as_str()
            .unwrap_or_default()
            .is_empty(),
        "trust waiting must surface provider-specific manual guidance: {body}"
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
    // Task 4：保留首个 pre_check turn 一次中断（trust retry 弧的确定性
    // Failed 检查点）；根规则材料由 provider 在 RuleAndMcpConfig 命令
    // 时机生成（生产同构），不再手工预写 AGENTS。
    let fixture = trust_route_fixture_with(
        true,
        RecipeFixtureOptions {
            fault_first_pre_check: true,
            ..RecipeFixtureOptions::default()
        },
    );
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

    // 中断窗口无需落盘任何根规则材料：真实链路中根规则由 provider 在
    // RuleAndMcpConfig 命令时机生成，重试的命令审计窗口将其作为本命令
    // 的 allowlisted 变更冻结（Task 4 生产同构）。

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

    // 最终 receipt 冻结 canonical root/policy/rule 三重身份（Task 4 起
    // policy digest 是末命令发布的真正文 digest，不再是自举桩）。
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
    .expect("provider-generated root rule entry");
    assert_eq!(receipt.rule_digest, rule_digest);
    let policy_artifact =
        crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::for_lc(
            fixture.paths.clone(),
            fixture.lc_id.clone(),
        )
        .get("project_0001")
        .expect("policy artifact read")
        .expect("published policy artifact");
    assert!(
        !policy_artifact.is_bootstrap_placeholder(),
        "the final command must publish the real aggregate policy, not keep the bootstrap stub"
    );
    assert_eq!(receipt.policy_digest, policy_artifact.digest);
    assert_eq!(receipt.finalized_at, policy_artifact.created_at);

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

/// 构造一个五步全部 Completed、operation 终态 Completed 的记录(时间戳
/// 系列早于 `create_running_initialization` 的 2026-09-29 系列)。
fn create_completed_initialization(paths: &ProductAppPaths, lc_id: &str, operation_id: &str) {
    let store = AggregateInitializationOperationStore::for_lc(paths.clone(), lc_id);
    let operation =
        crate::product::logical_codebase::aggregate_initialization::AggregateInitializationOperation::new(
            operation_id.to_string(),
            "project_0001".to_string(),
            crate::product::logical_codebase::aggregate_initialization::AggregateInitializationOperationInput {
                idempotency_key: format!("idem-{operation_id}"),
                manifest_revision: 1,
                policy_digest: "sha256:policy".to_string(),
                profile_evidence_digest: None,
                provider_context_root: paths
                    .logical_codebase_root("project_0001")
                    .join("aggregate-root"),
                provider: "claude_code".to_string(),
            },
            "2026-09-28T00:00:00Z".to_string(),
        );
    store.create_idempotent(operation).unwrap();
    store
        .mark_running(
            "project_0001",
            operation_id,
            "2026-09-28T00:00:10Z".to_string(),
        )
        .unwrap();
    for (index, step) in AggregateInitializationStepKind::V1.iter().enumerate() {
        store
            .mark_step_running(
                "project_0001",
                operation_id,
                *step,
                format!("sha256:{operation_id}-input-{index}"),
                format!("2026-09-28T00:01:{index:02}Z"),
            )
            .unwrap();
        store
            .checkpoint_step_output(
                "project_0001",
                operation_id,
                *step,
                format!("aggregate-initializations/{operation_id}/{}.json", step.as_str()),
                format!("2026-09-28T00:02:{index:02}Z"),
            )
            .unwrap();
        store
            .mark_step_completed(
                "project_0001",
                operation_id,
                *step,
                format!("2026-09-28T00:03:{index:02}Z"),
            )
            .unwrap();
    }
    store
        .finish_completed(
            "project_0001",
            operation_id,
            "2026-09-28T00:04:00Z".to_string(),
        )
        .unwrap();
}

#[tokio::test]
async fn aggregate_initialization_active_projection_returns_latest_operation() {
    let fixture = trust_route_fixture(false);
    let uri = format!(
        "/api/projects/project_0001/logical-codebases/{}/initializations/active",
        fixture.lc_id
    );

    // 无 operation:未初始化语义,404 + 稳定 code(与按 id GET 的 not_found 一致)。
    let (status, body) = get_json(&fixture.app, &uri).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        body["code"], "aggregate_initialization_operation_not_found",
 "{body}"
    );

    // 完成态 operation:active 投影必须返回 200 + completed,而不是把
    // `active` 当 operation_id 的 404——否则完成后的 LC 面板永远显示未初始化。
    create_completed_initialization(
        &fixture.paths,
        &fixture.lc_id,
        "aggregate_initialization_active_completed_0001",
    );
    let (status, body) = get_json(&fixture.app, &uri).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "completed", "{body}");
    assert_eq!(
        body["operation_id"], "aggregate_initialization_active_completed_0001",
 "{body}"
    );
    assert_eq!(body["completed_at"], "2026-09-28T00:04:00Z", "{body}");

    // 较新的 running operation 落盘后,active 返回 updated_at 最新的那个。
    create_running_initialization(
        &fixture.paths,
        &fixture.lc_id,
        "aggregate_initialization_active_running_0002",
    );
    let (status, body) = get_json(&fixture.app, &uri).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "running", "{body}");
    assert_eq!(
        body["operation_id"], "aggregate_initialization_active_running_0002",
 "{body}"
    );

    // 纯投影:零 provider 启动、零 trust 评估(与按 id GET 同约束)。
    assert_eq!(fixture.factory.audit().stream_launches(), 0);
    assert_eq!(
        fixture.gate.calls(),
        0,
        "GET active must not evaluate the trust gate"
    );
}
