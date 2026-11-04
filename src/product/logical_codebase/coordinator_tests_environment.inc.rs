// 环境前置（codegraph / 聚合根可写 / skills 链接层）的 coordinator 级行为锁。
// 纯包含片段：与 `coordinator_tests.inc.rs` 的 `mod tests` 共享作用域。

use crate::product::logical_codebase::environment_probe::{
    AggregateEnvironmentProbe, StubEnvironmentProbe,
};

/// 环境前置 fixture：可注入探针与 skills HOME，其余依赖沿用既有 fake。
struct EnvironmentPreconditionFixture {
    _temp: tempfile::TempDir,
    coordinator: AggregateInitializationCoordinator,
    provider: Arc<FakeProviderTurnDriver>,
    skills_calls: Arc<Mutex<Vec<String>>>,
    preflight_calls: Arc<Mutex<Vec<String>>>,
}

impl EnvironmentPreconditionFixture {
    fn new(
        probe: Option<Arc<dyn AggregateEnvironmentProbe>>,
        skills_home: Option<PathBuf>,
    ) -> Self {
        Self::build(probe, skills_home, None)
    }

    fn with_skills(
        probe: Option<Arc<dyn AggregateEnvironmentProbe>>,
        skills_home: Option<PathBuf>,
        skills: Arc<dyn AggregateSkillsPreparation>,
    ) -> Self {
        Self::build(probe, skills_home, Some(skills))
    }

    fn build(
        probe: Option<Arc<dyn AggregateEnvironmentProbe>>,
        skills_home: Option<PathBuf>,
        skills: Option<Arc<dyn AggregateSkillsPreparation>>,
    ) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let store = AggregateInitializationOperationStore::new(paths.clone());
        let skills_calls = Arc::new(Mutex::new(Vec::new()));
        let preflight_calls = Arc::new(Mutex::new(Vec::new()));
        let provider = Arc::new(FakeProviderTurnDriver::new());
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

        let skills = skills.unwrap_or_else(|| {
            Arc::new(FakeSkillsPreparation {
                calls: skills_calls.clone(),
            })
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
            store,
            skills,
            preflight,
            provider.clone(),
            clock,
        );
        if let Some(probe) = probe {
            coordinator = coordinator.with_environment_probe(probe);
        }
        if let Some(home) = skills_home {
            coordinator = coordinator.with_skills_home(home);
        }
        coordinator
            .begin(
                "aggregate_initialization_0001".to_string(),
                "project_0001",
                AggregateInitializationOperationInput {
                    idempotency_key: "0001".to_string(),
                    manifest_revision: manifest.membership_revision,
                    policy_digest: "sha256:policy".to_string(),
                    profile_evidence_digest: Some("sha256:profile".to_string()),
                    provider_context_root: manifest.provider_context_root.clone(),
                    provider: "claude_code".to_string(),
                },
            )
            .unwrap();
        Self {
            _temp: temp,
            coordinator,
            provider,
            skills_calls,
            preflight_calls,
        }
    }

    async fn execute(
        &self,
    ) -> Result<AggregateInitializationOperation, AggregateInitializationError> {
        self.coordinator
            .execute(
                "project_0001",
                "aggregate_initialization_0001",
                CancellationToken::new(),
            )
            .await
    }

    fn operation(&self) -> AggregateInitializationOperation {
        self.coordinator
            .get("project_0001", "aggregate_initialization_0001")
            .unwrap()
    }

    fn downstream_calls(&self) -> usize {
        self.skills_calls.lock().unwrap().len()
            + self.preflight_calls.lock().unwrap().len()
            + self.provider.turn_count()
    }
}

/// #4：聚合根可写探测。可写目录通过且不留 `.aria-probe-*` 残留；只读目录在
/// 预检阶段就报 `aggregate_root_not_writable`（不是通用的
/// `aggregate_preflight_failed`）。
#[cfg(unix)]
#[test]
fn preflight_rejects_read_only_aggregate_root_without_leaving_probe_residue() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let writable = temp.path().join("writable-root");
    std::fs::create_dir_all(&writable).unwrap();
    crate::product::logical_codebase::environment_probe::ensure_aggregate_root_writable(&writable)
        .unwrap();
    let residue = std::fs::read_dir(&writable)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert!(
        residue.is_empty(),
        "可写探测不得留下 `.aria-probe-*` 残留: {residue:?}"
    );

    let readonly = temp.path().join("readonly-root");
    std::fs::create_dir_all(&readonly).unwrap();
    std::fs::set_permissions(&readonly, std::fs::Permissions::from_mode(0o555)).unwrap();
    let manifest = LogicalCodebaseManifest::new("project_0001", readonly.clone(), Vec::new());
    let service = DeterministicAggregatePreflightService::new(ProductAppPaths::new(
        temp.path().join(".aria"),
    ));
    let result = service.inspect("project_0001", &manifest, &CancellationToken::new());
    std::fs::set_permissions(&readonly, std::fs::Permissions::from_mode(0o755)).unwrap();

    let error = result.expect_err("只读聚合根必须在预检阶段被拒绝");
    assert!(
        matches!(
            &error,
            AggregateInitializationError::EnvironmentPrecondition { code, .. }
                if code == "aggregate_root_not_writable"
        ),
        "expected aggregate_root_not_writable, got {error:?}"
    );
}

/// #5：`~/.claude/skills/pre-check/SKILL.md` 缺失 → `skills_commands_missing`，
/// **不是** `aggregate_machine_skills_failed`（machine_skills 是确定性代码，
/// 与 provider 登录无关）。
#[tokio::test]
async fn missing_skill_command_link_reports_skills_commands_missing() {
    let skills_home = tempfile::tempdir().unwrap();
    let fixture = EnvironmentPreconditionFixture::new(
        Some(Arc::new(StubEnvironmentProbe::available("1.6.0"))),
        Some(skills_home.path().to_path_buf()),
    );

    let error = fixture.execute().await.expect_err("缺链接层必须失败");
    assert!(
        matches!(
            &error,
            AggregateInitializationError::EnvironmentPrecondition { code, .. }
                if code == "skills_commands_missing"
        ),
        "expected skills_commands_missing, got {error:?}"
    );

    let operation = fixture.operation();
    assert_eq!(
        operation.status,
        AggregateInitializationOperationStatus::Failed
    );
    assert_eq!(
        operation
            .error
            .as_ref()
            .map(|record| record.reason_code.as_str()),
        Some("skills_commands_missing")
    );
    assert_eq!(operation.failed_step, None);
    // machine_skills 已完成；三个 provider turn 一次都没启动。
    assert_eq!(
        operation.steps[0].status,
        AggregateInitializationStepStatus::Completed
    );
    assert_eq!(fixture.provider.turn_count(), 0);
    assert!(
        operation.steps[1..]
            .iter()
            .all(|step| step.status == AggregateInitializationStepStatus::Pending)
    );
}

/// #6：探测失败时下游一步都不启动 —— 这也正是「索引构建未被触发」的可观测
/// 前提：handler 只在 `execute` 返回 `Ok` 之后才 spawn 索引构建。
#[tokio::test]
async fn environment_probe_failure_starts_no_downstream_step_or_index_build() {
    let fixture = EnvironmentPreconditionFixture::new(
        Some(Arc::new(StubEnvironmentProbe::codegraph_missing())),
        None,
    );

    let result = fixture.execute().await;
    assert!(
        result.is_err(),
        "探测失败时 execute 必须返回 Err（handler 据此不 spawn 索引构建）"
    );
    assert_eq!(
        fixture.downstream_calls(),
        0,
        "machine_skills / aggregate_preflight / provider turn 均不得启动"
    );
    let operation = fixture.operation();
    assert_eq!(operation.failed_step, None);
    assert!(
        operation
            .steps
            .iter()
            .all(|step| step.status == AggregateInitializationStepStatus::Pending)
    );
}

/// #7（记录侧）：带诊断的 skills 失败经 `error_record()` 仍产
/// `aggregate_machine_skills_failed`，并把结构化 `cause` 落到 durable 记录；
/// 旧 `SkillsPreparation` 的 `cause` 仍为 `None`（零回归）。
#[tokio::test]
async fn diagnosed_skills_failure_keeps_reason_code_and_carries_cause() {
    #[derive(Clone, Copy)]
    enum SkillsFailure {
        Diagnosed,
        Legacy,
    }

    struct FailingSkills(SkillsFailure);

    #[async_trait]
    impl AggregateSkillsPreparation for FailingSkills {
        async fn prepare_skills(
            &self,
            _project_id: &str,
            _operation_id: &str,
            _cancellation: CancellationToken,
        ) -> Result<MachineSkillsPreparation, AggregateInitializationError> {
            let reason = "cadence_skills_unavailable at clone: boom".to_string();
            Err(match self.0 {
                SkillsFailure::Diagnosed => {
                    AggregateInitializationError::SkillsPreparationDiagnosed {
                        reason,
                        retryable: true,
                        cause: "skills_unavailable_clone".to_string(),
                    }
                }
                SkillsFailure::Legacy => AggregateInitializationError::SkillsPreparation {
                    reason,
                    retryable: true,
                },
            })
        }
    }

    for (failure, expected_cause) in [
        (SkillsFailure::Diagnosed, Some("skills_unavailable_clone")),
        (SkillsFailure::Legacy, None),
    ] {
        let fixture = EnvironmentPreconditionFixture::with_skills(
            Some(Arc::new(StubEnvironmentProbe::available("1.6.0"))),
            None,
            Arc::new(FailingSkills(failure)),
        );
        fixture
            .execute()
            .await
            .expect_err("machine_skills 失败必须让 operation 失败");

        let operation = fixture.operation();
        let record = operation
            .error
            .as_ref()
            .expect("machine_skills 失败必须落 durable error record");
        assert_eq!(record.reason_code, "aggregate_machine_skills_failed");
        assert_eq!(record.cause.as_deref(), expected_cause);
        assert_eq!(
            operation.failed_step,
            Some(AggregateInitializationStepKind::MachineSkills)
        );
    }
}
