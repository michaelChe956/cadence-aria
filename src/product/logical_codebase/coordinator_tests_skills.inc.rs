// owner 2026-10-29：聚合初始化「跳过四条 Cadence-skills 命令」与
// 「重新初始化（聚合）」的 coordinator 级行为锁。全部只观察 durable
// operation 记录、provider 启动次数与政策发布调用，不依赖实现细节。

/// fixture 选项——聚合根是否已「做过 Cadence-skills 初始化」（预写两个
/// 判定文件）、是否显式要求强制重跑、是否装配跳过式政策发布面。
#[derive(Default)]
struct AggregateInitOptions {
    initialized_root: bool,
    full_recipe_requested: bool,
    policy: Option<Arc<dyn AggregatePolicyPublication>>,
}

fn aggregate_init_fixture_with(options: AggregateInitOptions) -> AggregateInitFixture {
    let temp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = AggregateInitializationOperationStore::new(paths.clone());
    let skills_calls = Arc::new(Mutex::new(Vec::new()));
    let preflight_calls = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(FakeProviderTurnDriver::new());

    // Persist a logical codebase manifest so the coordinator can load it.
    let mut manifest = LogicalCodebaseManifest::new(
        "project_0001",
        temp.path().join("aggregate-root"),
        Vec::new(),
    );
    manifest.created_at = CREATED_AT.to_string();
    manifest.updated_at = CREATED_AT.to_string();
    LogicalCodebaseStore::new(paths.clone())
        .save_manifest("project_0001", &manifest)
        .unwrap();
    if options.initialized_root {
        write_aggregate_skills_markers(&manifest.provider_context_root);
    }

    let skills: Arc<dyn AggregateSkillsPreparation> = Arc::new(FakeSkillsPreparation {
        calls: skills_calls.clone(),
    });
    let preflight: Arc<dyn AggregatePreflightService> = Arc::new(FakePreflightService::new(
        preflight_calls.clone(),
        manifest
            .provider_context_root
            .to_string_lossy()
            .into_owned(),
    ));
    let clock: Arc<Clock> = Arc::new(|| CREATED_AT.to_string());
    let mut coordinator = AggregateInitializationCoordinator::new(
        paths.clone(),
        store.clone(),
        skills,
        preflight,
        provider.clone(),
        clock,
    );
    if let Some(policy) = options.policy {
        coordinator = coordinator.with_policy_publication(policy);
    }

    // Begin an operation with the deterministic id the test references.
    let input = AggregateInitializationOperationInput {
        idempotency_key: "0001".to_string(),
        manifest_revision: manifest.membership_revision,
        policy_digest: "sha256:policy".to_string(),
        profile_evidence_digest: Some("sha256:profile".to_string()),
        provider_context_root: manifest.provider_context_root.clone(),
        provider: "claude_code".to_string(),
    };
    let operation_id = "aggregate_initialization_0001".to_string();
    if options.full_recipe_requested {
        coordinator
            .begin_reinitialization(operation_id, "project_0001", input)
            .unwrap();
    } else {
        coordinator
            .begin(operation_id, "project_0001", input)
            .unwrap();
    }

    AggregateInitFixture {
        _temp: temp,
        skills_calls,
        preflight_calls,
        provider,
        coordinator,
    }
}

/// 预写「聚合根已做过 Cadence-skills 初始化」的两个判定文件。
fn write_aggregate_skills_markers(root: &std::path::Path) {
    std::fs::create_dir_all(root).unwrap();
    std::fs::write(root.join("AGENTS.md"), "# 聚合根规则\n").unwrap();
    std::fs::create_dir_all(root.join(".claude/rules")).unwrap();
    std::fs::write(root.join(".claude/rules/language.md"), "# 语言规则\n").unwrap();
}

/// 记录跳过式发布调用的 fake 发布面。
struct FakePolicyPublication {
    calls: Arc<Mutex<Vec<String>>>,
}

impl FakePolicyPublication {
    fn new() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl AggregatePolicyPublication for FakePolicyPublication {
    async fn publish_skipped(
        &self,
        project_id: &str,
        operation_id: &str,
        lc_id: &str,
        canonical_root: &std::path::Path,
        decision: &AggregateSkillsInitialization,
    ) -> Result<(), AggregateInitializationError> {
        self.calls.lock().unwrap().push(format!(
            "{project_id}:{operation_id}:{lc_id}:{}:{}:{}",
            decision.kind.as_str(),
            decision.markers.len(),
            canonical_root.display()
        ));
        Ok(())
    }
}

/// 记录调用次数的 fake 发布面：第一次失败、之后成功（验证续跑不重新
/// 判定、政策发布失败必须落 Failed 且可显式续跑）。
struct FlakyPolicyPublication {
    attempts: Mutex<usize>,
}

impl FlakyPolicyPublication {
    fn new() -> Self {
        Self {
            attempts: Mutex::new(0),
        }
    }

    fn attempts(&self) -> usize {
        *self.attempts.lock().unwrap()
    }
}

#[async_trait]
impl AggregatePolicyPublication for FlakyPolicyPublication {
    async fn publish_skipped(
        &self,
        _project_id: &str,
        _operation_id: &str,
        _lc_id: &str,
        _canonical_root: &std::path::Path,
        _decision: &AggregateSkillsInitialization,
    ) -> Result<(), AggregateInitializationError> {
        let mut attempts = self.attempts.lock().unwrap();
        *attempts += 1;
        if *attempts == 1 {
            return Err(AggregateInitializationError::PolicyPublication {
                reason: "injected policy publication failure".to_string(),
                retryable: true,
            });
        }
        Ok(())
    }
}

fn step_statuses(
    operation: &crate::product::logical_codebase::AggregateInitializationOperation,
) -> Vec<&'static str> {
    operation
        .steps
        .iter()
        .map(|step| step.status.as_str())
        .collect()
}

/// 验收 ①④：聚合根已做过 Cadence-skills 初始化（`AGENTS.md` 且
/// `.claude/rules/language.md` 存在）→ 三个 provider-turn 步骤被判跳过、
/// 零 provider 启动，只做聚合政策发布；判定依据可从 operation 记录读到。
#[tokio::test]
async fn initialized_aggregate_root_skips_the_four_skills_commands() {
    let policy = Arc::new(FakePolicyPublication::new());
    let fixture = aggregate_init_fixture_with(AggregateInitOptions {
        initialized_root: true,
        full_recipe_requested: false,
        policy: Some(policy.clone()),
    });

    let operation = fixture
        .coordinator()
        .execute(
            "project_0001",
            "aggregate_initialization_0001",
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(
        operation.status,
        AggregateInitializationOperationStatus::Completed
    );
    assert_eq!(
        step_statuses(&operation),
        vec!["completed", "completed", "skipped", "skipped", "skipped"],
        "an initialized root must skip exactly the four-command steps"
    );
    assert!(
        fixture.provider().calls().is_empty(),
        "skipped steps must never spawn a provider turn"
    );
    assert_eq!(
        fixture.calls(),
        vec!["machine_skills", "aggregate_preflight"],
        "deterministic steps still run; provider turns do not"
    );

    // ④ 判定依据 durable 可读：判定类别 + reason + 两个判定文件摘要。
    let decision = operation
        .skills_initialization
        .clone()
        .expect("skip decision must be recorded on the operation");
    assert_eq!(
        decision.kind,
        crate::product::logical_codebase::aggregate_initialization::AggregateSkillsInitializationKind::Skipped
    );
    assert_eq!(
        decision.reason_code,
        crate::product::logical_codebase::aggregate_initialization::AGGREGATE_SKILLS_SKIPPED_REASON
    );
    assert_eq!(
        decision
            .markers
            .iter()
            .map(|marker| marker.relative_path.as_str())
            .collect::<Vec<_>>(),
        vec!["AGENTS.md", ".claude/rules/language.md"]
    );
    assert_eq!(
        decision.markers[0].digest,
        format!("sha256:{:x}", Sha256::digest("# 聚合根规则\n".as_bytes())),
        "marker digests must freeze the actual aggregate-root files"
    );
    for step in &operation.steps[2..] {
        assert_eq!(
            step.skip_reason.as_deref(),
            Some(
                crate::product::logical_codebase::aggregate_initialization::AGGREGATE_SKILLS_SKIPPED_REASON
            ),
            "every skipped step must carry the decision reason"
        );
        assert!(
            step.input_digest.is_none() && step.output_artifact_ref.is_none(),
            "a skipped step must not pretend to have run"
        );
    }

    // 政策发布被调用恰好一次，且携带同一判定依据。
    let publication_calls = policy.calls();
    assert_eq!(publication_calls.len(), 1);
    assert!(
        publication_calls[0].starts_with("project_0001:aggregate_initialization_0001:"),
        "publication must bind the operation: {publication_calls:?}"
    );
    assert!(
        publication_calls[0].contains(":skipped:2:"),
        "publication must carry the skipped decision and both markers: {publication_calls:?}"
    );
}

/// 验收 ②：聚合根未初始化 → 仍走完整五步（三个 provider turn 全部启动），
/// 不落跳过判定。
#[tokio::test]
async fn uninitialized_aggregate_root_keeps_the_full_five_step_recipe() {
    let policy = Arc::new(FakePolicyPublication::new());
    let fixture = aggregate_init_fixture_with(AggregateInitOptions {
        initialized_root: false,
        full_recipe_requested: false,
        policy: Some(policy.clone()),
    });

    let operation = fixture
        .coordinator()
        .execute(
            "project_0001",
            "aggregate_initialization_0001",
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(
        operation.status,
        AggregateInitializationOperationStatus::Completed
    );
    assert_eq!(
        step_statuses(&operation),
        vec!["completed"; 5],
        "an uninitialized root must run the full five-step recipe"
    );
    assert_eq!(
        fixture.provider().calls(),
        vec!["pre_check", "rule_and_mcp_config", "openspec_and_examples"]
    );
    assert!(
        operation.skills_initialization.is_none(),
        "no skip decision may be recorded when the commands really ran"
    );
    assert!(
        policy.calls().is_empty(),
        "the full recipe publishes policy through its final command turn"
    );
}

/// 验收 ③：显式「重新初始化（聚合）」→ 即便聚合根已初始化也跑完整五步。
#[tokio::test]
async fn forced_reinitialization_runs_every_command_on_an_initialized_root() {
    let policy = Arc::new(FakePolicyPublication::new());
    let fixture = aggregate_init_fixture_with(AggregateInitOptions {
        initialized_root: true,
        full_recipe_requested: true,
        policy: Some(policy.clone()),
    });

    let operation = fixture
        .coordinator()
        .execute(
            "project_0001",
            "aggregate_initialization_0001",
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert!(operation.full_recipe_requested);
    assert_eq!(
        operation.status,
        AggregateInitializationOperationStatus::Completed
    );
    assert_eq!(
        step_statuses(&operation),
        vec!["completed"; 5],
        "a forced reinitialization must not be affected by the skip detection"
    );
    assert_eq!(
        fixture.provider().calls(),
        vec!["pre_check", "rule_and_mcp_config", "openspec_and_examples"]
    );
    let decision = operation
        .skills_initialization
        .clone()
        .expect("the forced decision must be recorded for auditability");
    assert_eq!(
        decision.kind,
        crate::product::logical_codebase::aggregate_initialization::AggregateSkillsInitializationKind::Forced
    );
    assert_eq!(
        decision.reason_code,
        crate::product::logical_codebase::aggregate_initialization::AGGREGATE_SKILLS_FORCED_REASON
    );
    assert!(
        policy.calls().is_empty(),
        "the forced path publishes policy through its final command turn"
    );
}

/// 跳过判定存在但没有政策发布面 → fail-closed（绝不静默跳过政策发布）。
#[tokio::test]
async fn skipped_initialization_without_a_publication_path_fails_closed() {
    let fixture = aggregate_init_fixture_with(AggregateInitOptions {
        initialized_root: true,
        full_recipe_requested: false,
        policy: None,
    });

    let error = fixture
        .coordinator()
        .execute(
            "project_0001",
            "aggregate_initialization_0001",
            CancellationToken::new(),
        )
        .await
        .expect_err("a skipped initialization must not silently publish nothing");
    assert!(matches!(
        error,
        AggregateInitializationError::PolicyPublication { retryable: false, .. }
    ));

    let operation = fixture
        .coordinator()
        .get("project_0001", "aggregate_initialization_0001")
        .unwrap();
    assert_eq!(
        operation.status,
        AggregateInitializationOperationStatus::Failed
    );
    assert_eq!(
        operation.error.as_ref().map(|error| error.reason_code.as_str()),
        Some("aggregate_policy_publication_failed")
    );
    assert_eq!(
        step_statuses(&operation),
        vec!["completed", "completed", "skipped", "skipped", "skipped"]
    );
    assert!(fixture.provider().calls().is_empty());
    // 判定依据在失败态同样可读（跳过不是静默的）。
    assert!(
        operation
            .skills_initialization
            .as_ref()
            .is_some_and(|decision| decision.is_skipped())
    );
}

/// 跳过式政策发布失败后显式续跑：不重新判定（沿用冻结的跳过依据）、
/// 不重复跑确定性步骤、成功即 Completed。
#[tokio::test]
async fn skipped_initialization_resume_reuses_the_frozen_decision() {
    let policy = Arc::new(FlakyPolicyPublication::new());
    let fixture = aggregate_init_fixture_with(AggregateInitOptions {
        initialized_root: true,
        full_recipe_requested: false,
        policy: Some(policy.clone()),
    });

    assert!(
        fixture
            .coordinator()
            .execute(
                "project_0001",
                "aggregate_initialization_0001",
                CancellationToken::new(),
            )
            .await
            .is_err()
    );

    let resumed = fixture
        .coordinator()
        .execute_remaining(
            "project_0001",
            "aggregate_initialization_0001",
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        resumed.status,
        AggregateInitializationOperationStatus::Completed
    );
    assert_eq!(
        step_statuses(&resumed),
        vec!["completed", "completed", "skipped", "skipped", "skipped"]
    );
    assert_eq!(
        policy.attempts(),
        2,
        "resume must retry exactly the publication, not re-decide"
    );
    assert_eq!(
        fixture.calls(),
        vec!["machine_skills", "aggregate_preflight"],
        "deterministic steps must not run twice"
    );
}

/// 已有命令审计事实的 operation（典型：旧版本中断后显式续跑）绝不改判
/// 为跳过：命令已经跑过是「没必要跑」的反证，必须走完整五步。
#[tokio::test]
async fn audited_commands_close_the_skip_window() {
    struct AuditedCommandsPublication;

    #[async_trait]
    impl AggregatePolicyPublication for AuditedCommandsPublication {
        async fn publish_skipped(
            &self,
            _project_id: &str,
            _operation_id: &str,
            _lc_id: &str,
            _canonical_root: &std::path::Path,
            _decision: &AggregateSkillsInitialization,
        ) -> Result<(), AggregateInitializationError> {
            panic!("a skip decision must not be reached once a command was audited");
        }

        fn has_audited_commands(
            &self,
            _project_id: &str,
            _operation_id: &str,
            _lc_id: &str,
        ) -> Result<bool, AggregateInitializationError> {
            Ok(true)
        }
    }

    let fixture = aggregate_init_fixture_with(AggregateInitOptions {
        initialized_root: true,
        full_recipe_requested: false,
        policy: Some(Arc::new(AuditedCommandsPublication)),
    });

    let operation = fixture
        .coordinator()
        .execute(
            "project_0001",
            "aggregate_initialization_0001",
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(
        step_statuses(&operation),
        vec!["completed"; 5],
        "an operation that already audited commands must finish the full recipe"
    );
    assert!(operation.skills_initialization.is_none());
    assert_eq!(fixture.provider().turn_count(), 3);
}
