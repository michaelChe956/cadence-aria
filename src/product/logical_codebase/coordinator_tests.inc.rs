#[cfg(test)]
mod tests {
    use super::*;
    use crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
    use crate::cross_cutting::provider_registry::ProviderRegistry;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::aggregate_initialization::AggregateInitializationOperationInput;
    use crate::product::logical_codebase::provider_gateway::ResumeEvidenceState;
    use crate::product::logical_codebase::types::LogicalRepositoryId;
    use crate::product::logical_codebase::{
        CodebaseMemberRecord, GatewayRunAudit, LogicalCodebaseProviderGateway,
        LogicalCodebaseStore, PolicyTarget, PolicyTargetResolver, ProviderCapability,
        ProviderCapabilitySource, ProviderDialect, ProviderGatewayError, ProviderRef,
        ProviderRefType, RepositoryCheckoutId, RepositoryCheckoutRecord, RepositorySourceIdentity,
        SessionLaunchRequest, SessionPolicyAction,
    };
    use crate::product::models::ProviderName;
    use std::path::Path;
    use std::sync::Mutex;
    use uuid::Uuid;

    const CREATED_AT: &str = "2026-08-09T00:00:00Z";

    struct FakeProviderTurnDriver {
        calls: Mutex<Vec<String>>,
    }

    impl FakeProviderTurnDriver {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn turn_count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }

    #[async_trait]
    impl AggregateProviderTurnDriver for FakeProviderTurnDriver {
        async fn run_turn(
            &self,
            _project_id: &str,
            _operation_id: &str,
            step: AggregateInitializationStepKind,
            _preflight: &AggregatePreflightSnapshot,
            _cancellation: CancellationToken,
        ) -> Result<String, AggregateInitializationError> {
            self.calls.lock().unwrap().push(step.as_str().to_string());
            Ok(format!("{} summary", step.as_str()))
        }
    }

    struct FakeSkillsPreparation {
        calls: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl AggregateSkillsPreparation for FakeSkillsPreparation {
        async fn prepare_skills(
            &self,
            _project_id: &str,
            _operation_id: &str,
            _cancellation: CancellationToken,
        ) -> Result<MachineSkillsPreparation, AggregateInitializationError> {
            self.calls
                .lock()
                .unwrap()
                .push("machine_skills".to_string());
            Ok(MachineSkillsPreparation {
                source_digest: "sha256:source".to_string(),
                link_digest: "sha256:link".to_string(),
                skills_root: PathBuf::from("/skills"),
                warnings: Vec::new(),
            })
        }
    }

    struct FakePreflightService {
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl AggregatePreflightService for FakePreflightService {
        fn inspect(
            &self,
            _project_id: &str,
            _manifest: &LogicalCodebaseManifest,
            _cancellation: &CancellationToken,
        ) -> Result<AggregatePreflightSnapshot, AggregateInitializationError> {
            self.calls
                .lock()
                .unwrap()
                .push("aggregate_preflight".to_string());
            Ok(AggregatePreflightSnapshot {
                aggregate_root: "/aggregate-root".to_string(),
                index_excludes_assets: true,
                members: Vec::new(),
                manifest_revision: 1,
                manifest_digest: "sha256:manifest".to_string(),
            })
        }
    }

    struct AggregateInitFixture {
        _temp: tempfile::TempDir,
        skills_calls: Arc<Mutex<Vec<String>>>,
        preflight_calls: Arc<Mutex<Vec<String>>>,
        provider: Arc<FakeProviderTurnDriver>,
        coordinator: AggregateInitializationCoordinator,
    }

    impl AggregateInitFixture {
        fn provider(&self) -> &FakeProviderTurnDriver {
            &self.provider
        }

        fn coordinator(&self) -> &AggregateInitializationCoordinator {
            &self.coordinator
        }

        fn calls(&self) -> Vec<String> {
            let mut calls = self.skills_calls.lock().unwrap().clone();
            calls.extend(self.preflight_calls.lock().unwrap().clone());
            calls.extend(self.provider.calls());
            calls
        }
    }

    fn aggregate_init_fixture() -> AggregateInitFixture {
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

        let skills: Arc<dyn AggregateSkillsPreparation> = Arc::new(FakeSkillsPreparation {
            calls: skills_calls.clone(),
        });
        let preflight: Arc<dyn AggregatePreflightService> = Arc::new(FakePreflightService {
            calls: preflight_calls.clone(),
        });
        let clock: Arc<Clock> = Arc::new(|| CREATED_AT.to_string());
        let coordinator = AggregateInitializationCoordinator::new(
            paths.clone(),
            store.clone(),
            skills,
            preflight,
            provider.clone(),
            clock,
        );

        // Begin an operation with the deterministic id the test references.
        let input = AggregateInitializationOperationInput {
            idempotency_key: "0001".to_string(),
            manifest_revision: manifest.membership_revision,
            policy_digest: "sha256:policy".to_string(),
            profile_evidence_digest: Some("sha256:profile".to_string()),
            provider_context_root: manifest.provider_context_root.clone(),
            provider: "claude_code".to_string(),
        };
        coordinator
            .begin(
                "aggregate_initialization_0001".to_string(),
                "project_0001",
                input,
            )
            .unwrap();

        AggregateInitFixture {
            _temp: temp,
            skills_calls,
            preflight_calls,
            provider,
            coordinator,
        }
    }

    #[tokio::test]
    async fn machine_skills_and_preflight_run_before_any_provider_turn() {
        let fixture = aggregate_init_fixture();
        fixture
            .coordinator()
            .execute(
                "project_0001",
                "aggregate_initialization_0001",
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(
            fixture.calls(),
            vec![
                "machine_skills",
                "aggregate_preflight",
                "pre_check",
                "rule_and_mcp_config",
                "openspec_and_examples",
            ]
        );
        assert_eq!(fixture.provider().turn_count(), 3);
    }

    #[tokio::test]
    async fn execute_completes_all_five_steps_in_strict_order() {
        let fixture = aggregate_init_fixture();
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
            operation
                .steps
                .iter()
                .map(|step| (step.step_id.as_str(), step.status.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("machine_skills", "completed"),
                ("aggregate_preflight", "completed"),
                ("pre_check", "completed"),
                ("rule_and_mcp_config", "completed"),
                ("openspec_and_examples", "completed"),
            ]
        );
        assert!(operation.completed_at.is_some());
    }

    #[tokio::test]
    async fn provider_failure_leaves_subsequent_steps_pending_and_marks_operation_failed() {
        struct FailingProvider;
        #[async_trait]
        impl AggregateProviderTurnDriver for FailingProvider {
            async fn run_turn(
                &self,
                _project_id: &str,
                _operation_id: &str,
                step: AggregateInitializationStepKind,
                _preflight: &AggregatePreflightSnapshot,
                _cancellation: CancellationToken,
            ) -> Result<String, AggregateInitializationError> {
                if step == AggregateInitializationStepKind::RuleAndMcpConfig {
                    return Err(AggregateInitializationError::ProviderTurn {
                        step,
                        reason: "rule/mcp turn rejected".to_string(),
                        retryable: true,
                    });
                }
                Ok("summary".to_string())
            }
        }

        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let store = AggregateInitializationOperationStore::new(paths.clone());
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

        let skills: Arc<dyn AggregateSkillsPreparation> = Arc::new(FakeSkillsPreparation {
            calls: Arc::new(Mutex::new(Vec::new())),
        });
        let preflight: Arc<dyn AggregatePreflightService> = Arc::new(FakePreflightService {
            calls: Arc::new(Mutex::new(Vec::new())),
        });
        let provider: Arc<dyn AggregateProviderTurnDriver> = Arc::new(FailingProvider);
        let clock: Arc<Clock> = Arc::new(|| CREATED_AT.to_string());
        let coordinator = AggregateInitializationCoordinator::new(
            paths.clone(),
            store.clone(),
            skills,
            preflight,
            provider,
            clock,
        );
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

        let result = coordinator
            .execute(
                "project_0001",
                "aggregate_initialization_0001",
                CancellationToken::new(),
            )
            .await;
        assert!(matches!(
            result,
            Err(AggregateInitializationError::ProviderTurn { .. })
        ));

        let operation = coordinator
            .get("project_0001", "aggregate_initialization_0001")
            .unwrap();
        assert_eq!(
            operation.status,
            AggregateInitializationOperationStatus::Failed
        );
        assert_eq!(
            operation.failed_step,
            Some(AggregateInitializationStepKind::RuleAndMcpConfig)
        );
        // rule_and_mcp_config failed; openspec_and_examples stays pending.
        let openspec = operation
            .steps
            .iter()
            .find(|step| step.step_id == AggregateInitializationStepKind::OpenspecAndExamples)
            .unwrap();
        assert_eq!(openspec.status.as_str(), "pending");
    }

    #[tokio::test]
    async fn cancellation_fails_running_operation_and_can_be_recovered() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let store = AggregateInitializationOperationStore::new(paths.clone());
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

        struct CountingProvider {
            count: Mutex<u32>,
        }
        #[async_trait]
        impl AggregateProviderTurnDriver for CountingProvider {
            async fn run_turn(
                &self,
                _project_id: &str,
                _operation_id: &str,
                _step: AggregateInitializationStepKind,
                _preflight: &AggregatePreflightSnapshot,
                _cancellation: CancellationToken,
            ) -> Result<String, AggregateInitializationError> {
                let mut count = self.count.lock().unwrap();
                *count += 1;
                Ok("summary".to_string())
            }
        }

        let skills: Arc<dyn AggregateSkillsPreparation> = Arc::new(FakeSkillsPreparation {
            calls: Arc::new(Mutex::new(Vec::new())),
        });
        let preflight: Arc<dyn AggregatePreflightService> = Arc::new(FakePreflightService {
            calls: Arc::new(Mutex::new(Vec::new())),
        });
        let provider: Arc<dyn AggregateProviderTurnDriver> = Arc::new(CountingProvider {
            count: Mutex::new(0),
        });
        let clock: Arc<Clock> = Arc::new(|| CREATED_AT.to_string());
        let coordinator = AggregateInitializationCoordinator::new(
            paths.clone(),
            store.clone(),
            skills,
            preflight,
            provider,
            clock,
        );
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

        let token = CancellationToken::new();
        token.cancel();
        let result = coordinator
            .execute("project_0001", "aggregate_initialization_0001", token)
            .await;
        assert!(matches!(
            result,
            Err(AggregateInitializationError::Cancelled)
        ));

        let operation = coordinator
            .get("project_0001", "aggregate_initialization_0001")
            .unwrap();
        assert!(
            matches!(
                operation.status,
                AggregateInitializationOperationStatus::Failed
                    | AggregateInitializationOperationStatus::Cancelled
            ),
            "cancelled execution must leave a terminal record"
        );
    }

    // ---- Task 16: provider step 经 gateway 启动 + GitFinalize 调用图切断 ----

    /// 测试用 capability source:固定 Claude Code capability,version 与 resume
    /// 能力可调,用于 gateway 复验。聚合 provider turn 固定 Claude Code(Codex
    /// danger-full-access 被 gateway 路由级阻断)。
    struct StaticCapabilitySource {
        version: Mutex<String>,
        resume: Mutex<ResumeEvidenceState>,
    }

    impl StaticCapabilitySource {
        fn new(version: &str) -> Self {
            Self {
                version: Mutex::new(version.to_string()),
                resume: Mutex::new(ResumeEvidenceState::Confirmed),
            }
        }
    }

    impl ProviderCapabilitySource for StaticCapabilitySource {
        fn require_supported(
            &self,
            provider: &ProviderRef,
            _action: SessionPolicyAction,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            Ok(ProviderCapability {
                provider_type: provider.provider_type,
                version: self.version.lock().unwrap().clone(),
                adapter_dialect: match provider.provider_type {
                    ProviderRefType::ClaudeCode => ProviderDialect::ClaudeCodeCliV1,
                    ProviderRefType::Codex => ProviderDialect::CodexCliV1,
                },
                capability_snapshot_ref: provider.capability_snapshot_ref.clone(),
                resume_evidence: *self.resume.lock().unwrap(),
            })
        }
    }

    /// 测试用 target resolver:直接返回请求中的 target(聚合根路径已由 fixture
    /// 真实创建)。spawn 前 canonical 复验由 gateway 内部完成。
    struct PassthroughTargetResolver;

    impl PolicyTargetResolver for PassthroughTargetResolver {
        fn resolve_and_revalidate(
            &self,
            request: &SessionLaunchRequest,
        ) -> Result<PolicyTarget, ProviderGatewayError> {
            Ok(request.target.clone())
        }
    }

    /// 测试用 streaming adapter:记录 start 调用次数并立即完成会话。
    struct CountingStreamingAdapter {
        start_count: std::sync::atomic::AtomicUsize,
    }

    impl CountingStreamingAdapter {
        fn new() -> Self {
            Self {
                start_count: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn start_count(&self) -> usize {
            self.start_count.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl crate::cross_cutting::streaming_provider::StreamingProviderAdapter
        for CountingStreamingAdapter
    {
        async fn start(
            &self,
            _input: crate::cross_cutting::streaming_provider::StreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<
            crate::cross_cutting::streaming_provider::ProviderSession,
            crate::cross_cutting::provider_adapter::ProviderAdapterError,
        > {
            self.start_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let (_event_tx, events) = tokio::sync::mpsc::channel(1);
            let (commands, _command_rx) = tokio::sync::mpsc::channel(1);
            Ok(crate::cross_cutting::streaming_provider::ProviderSession { events, commands })
        }
    }

    struct StubSyncAdapter;

    impl crate::cross_cutting::provider_adapter::ProviderAdapter for StubSyncAdapter {
        fn run(
            &self,
            _input: &crate::protocol::contracts::AdapterInput,
        ) -> Result<
            crate::protocol::contracts::AdapterOutput,
            crate::cross_cutting::provider_adapter::ProviderAdapterError,
        > {
            use crate::protocol::contracts::TimeoutStatus;
            Ok(crate::protocol::contracts::AdapterOutput {
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
                structured_output: None,
                files_modified: Vec::new(),
                duration_ms: 0,
                timeout_status: TimeoutStatus::NotTimedOut,
            })
        }
    }

    fn always_available_gate() -> Arc<ProviderAvailabilityGate> {
        use crate::cross_cutting::provider_availability_gate::ProviderHealthSource;
        use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
        use chrono::Utc;

        struct AlwaysHealthy(Arc<ProviderHealthSnapshot>);
        impl ProviderHealthSource for AlwaysHealthy {
            fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
                self.0.clone()
            }
            fn degraded(&self) -> bool {
                false
            }
        }

        let checked_at = Utc::now();
        let snapshot = Arc::new(ProviderHealthSnapshot {
            schema_version: 1,
            generation: 1,
            checked_at,
            providers: [ProviderName::ClaudeCode, ProviderName::Codex]
                .into_iter()
                .map(|provider| ProviderHealthEntry {
                    provider,
                    command: "stub".to_string(),
                    available: true,
                    version: Some("1.0".to_string()),
                    reason_code: None,
                    reason: None,
                    checked_at,
                })
                .collect(),
        });
        Arc::new(ProviderAvailabilityGate::new(Arc::new(AlwaysHealthy(
            snapshot,
        ))))
    }

    /// gateway-backed 聚合初始化 fixture:把 `GatewayBackedAggregateProviderTurnDriver`
    /// 注入 coordinator,使三个 provider turn 唯一经 gateway 启动。共享
    /// `GatewayRunAudit` 与 `CountingStreamingAdapter` 供断言。
    struct GatewayAggregateFixture {
        _temp: tempfile::TempDir,
        audit: Arc<GatewayRunAudit>,
        streaming_adapter: Arc<CountingStreamingAdapter>,
        coordinator: AggregateInitializationCoordinator,
    }

    impl GatewayAggregateFixture {
        fn coordinator(&self) -> &AggregateInitializationCoordinator {
            &self.coordinator
        }

        fn gateway_audit(&self) -> Arc<GatewayRunAudit> {
            self.audit.clone()
        }

        fn streaming_start_count(&self) -> usize {
            self.streaming_adapter.start_count()
        }
    }

    fn gateway_aggregate_fixture() -> GatewayAggregateFixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let store = AggregateInitializationOperationStore::new(paths.clone());

        // 聚合根是真实目录(aggregate_preflight + gateway spawn 前 canonicalize 需要它存在且非 git)。
        let aggregate_root = temp.path().join("aggregate-root");
        std::fs::create_dir_all(&aggregate_root).unwrap();

        let manifest =
            LogicalCodebaseManifest::new("project_0001", aggregate_root.clone(), Vec::new());
        LogicalCodebaseStore::new(paths.clone())
            .save_manifest("project_0001", &manifest)
            .unwrap();

        // 安装 bootstrap policy(gateway validate 需要 policy artifact)。
        crate::product::logical_codebase::provider_gateway::ensure_bootstrap_policy(
            &paths, &manifest,
        )
        .unwrap();

        let audit = Arc::new(GatewayRunAudit::new());
        let streaming_adapter = Arc::new(CountingStreamingAdapter::new());
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderName::ClaudeCode, streaming_adapter.clone());
        let gateway = Arc::new(LogicalCodebaseProviderGateway::with_audit(
            crate::product::logical_codebase::AggregatePolicyArtifactStore::new(paths.clone()),
            Arc::new(StaticCapabilitySource::new("1.4.0")),
            Arc::new(PassthroughTargetResolver),
            Arc::new(registry),
            Arc::new(StubSyncAdapter),
            always_available_gate(),
            audit.clone(),
        ));

        let skills: Arc<dyn AggregateSkillsPreparation> = Arc::new(FakeSkillsPreparation {
            calls: Arc::new(Mutex::new(Vec::new())),
        });
        // 用真实 `DeterministicAggregatePreflightService` 以产出可 canonicalize 的聚合根快照,
        // 使 gateway spawn 前 cwd 复验能通过。
        let preflight: Arc<dyn AggregatePreflightService> =
            Arc::new(DeterministicAggregatePreflightService::new(paths.clone()));
        let provider: Arc<dyn AggregateProviderTurnDriver> = Arc::new(
            GatewayBackedAggregateProviderTurnDriver::claude_code(gateway, "cap_claude_code_1_4_0"),
        );
        let clock: Arc<Clock> = Arc::new(|| CREATED_AT.to_string());
        let coordinator = AggregateInitializationCoordinator::new(
            paths.clone(),
            store.clone(),
            skills,
            preflight,
            provider,
            clock,
        );

        let input = AggregateInitializationOperationInput {
            idempotency_key: "0001".to_string(),
            manifest_revision: manifest.membership_revision,
            policy_digest: "sha256:policy".to_string(),
            profile_evidence_digest: Some("sha256:profile".to_string()),
            provider_context_root: manifest.provider_context_root.clone(),
            provider: "claude_code".to_string(),
        };
        coordinator
            .begin(
                "aggregate_initialization_0001".to_string(),
                "project_0001",
                input,
            )
            .unwrap();

        GatewayAggregateFixture {
            _temp: temp,
            audit,
            streaming_adapter,
            coordinator,
        }
    }

    #[tokio::test]
    async fn provider_steps_use_gateway_to_launch_three_streaming_turns() {
        let fixture = gateway_aggregate_fixture();
        fixture
            .coordinator()
            .execute(
                "project_0001",
                "aggregate_initialization_0001",
                CancellationToken::new(),
            )
            .await
            .unwrap();

        // 三个 provider turn(pre_check/rule_and_mcp_config/openspec_and_examples)
        // 唯一经 gateway 启动,故 stream_launches()==3。machine_skills 与
        // aggregate_preflight 是确定性 Cadence 代码,不产生 gateway 启动。
        assert_eq!(fixture.gateway_audit().stream_launches(), 3);
        assert_eq!(fixture.streaming_start_count(), 3);
        assert!(fixture.gateway_audit().all_have_policy_digest());
    }

    #[test]
    fn launch_request_target_is_aggregate_root_and_resolvable_by_production_resolver() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let aggregate_root = temp.path().join("aggregate-root");
        std::fs::create_dir_all(&aggregate_root).unwrap();

        let gateway = Arc::new(LogicalCodebaseProviderGateway::new(
            crate::product::logical_codebase::AggregatePolicyArtifactStore::new(paths.clone()),
            Arc::new(StaticCapabilitySource::new("1.4.0")),
            Arc::new(PassthroughTargetResolver),
            Arc::new(ProviderRegistry::new()),
            Arc::new(StubSyncAdapter),
            always_available_gate(),
        ));
        let driver =
            GatewayBackedAggregateProviderTurnDriver::claude_code(gateway, "cap_managed_snapshot");
        let request = driver.launch_request("project_0001", &aggregate_root);

        // 聚合根 planning 只读 target:logical/checkout id 均为空。
        assert!(request.target.logical_repository_id.is_empty());
        assert!(request.target.checkout_id.is_empty());

        // 生产 resolver 可解析(仅 canonicalize 聚合根目录),防止占位 checkout 目标回归。
        let resolver =
            crate::product::logical_codebase::ProductionPolicyTargetResolver::new(paths);
        let resolved = resolver.resolve_and_revalidate(&request).unwrap();
        assert!(resolved.logical_repository_id.is_empty());
        assert_eq!(
            resolved.worktree,
            std::fs::canonicalize(&aggregate_root).unwrap()
        );
    }

    #[test]
    fn aggregate_coordinator_isolation_locked_against_single_repository_persistence_and_git_finalize()
     {
        // 隔离回归门:coordinator 生产代码(非测试、非 doc comment)不得引用
        // 单仓持久化层或单仓 git 终结点,保证聚合模式不进入成员仓 git 调用图。
        // 主文件与按职责拆出的 `.inc.rs` 生产模块都在扫描范围内;测试模块
        // 与 doc comment 行被跳过以避免自指。
        let production_sources = [
            include_str!("aggregate_initialization_coordinator.rs"),
            include_str!("coordinator_lifecycle.inc.rs"),
            include_str!("coordinator_provider_turn.inc.rs"),
            include_str!("coordinator_preflight.inc.rs"),
            include_str!("coordinator_profile.inc.rs"),
        ];
        let forbidden = [
            "RepositoryPersistence",
            "git_finalize",
            "RepositoryRegistrationCoordinator",
        ];
        for source in production_sources {
            let mut in_test_module = false;
            for line in source.lines() {
                if line.trim_start().starts_with("#[cfg(test)]") {
                    in_test_module = true;
                }
                if in_test_module {
                    continue;
                }
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") {
                    continue;
                }
                for token in forbidden {
                    assert!(
                        !line.contains(token),
                        "aggregate coordinator production code must not reference {token}: {line}"
                    );
                }
            }
        }
    }

    #[test]
    fn aggregate_asset_publisher_only_accepts_aria_aggregate_paths() {
        let publisher = AggregateAssetPublisher::new();
        publisher
            .publish("aggregate_initialization_0001", ".aria/aggregate/CLAUDE.md")
            .unwrap();
        publisher
            .publish("aggregate_initialization_0001", ".aria/aggregate/mcp.json")
            .unwrap();
        publisher
            .publish(
                "aggregate_initialization_0001",
                ".aria/aggregate/openspec-examples.json",
            )
            .unwrap();
        assert_eq!(
            publisher.published_paths(),
            vec![
                ".aria/aggregate/CLAUDE.md",
                ".aria/aggregate/mcp.json",
                ".aria/aggregate/openspec-examples.json",
            ]
        );
    }

    #[test]
    fn aggregate_asset_publisher_rejects_member_repository_and_escape_paths() {
        let publisher = AggregateAssetPublisher::new();
        // 成员仓路径:fail-closed。
        assert!(
            publisher
                .publish(
                    "aggregate_initialization_0001",
                    "members/repo_0001/CLAUDE.md"
                )
                .is_err()
        );
        // 父目录逃逸:fail-closed。
        assert!(
            publisher
                .publish(
                    "aggregate_initialization_0001",
                    ".aria/aggregate/../../../etc/passwd"
                )
                .is_err()
        );
        // 绝对路径:fail-closed。
        assert!(
            publisher
                .publish("aggregate_initialization_0001", "/etc/passwd")
                .is_err()
        );
        // 仅 `.aria/aggregate` 目录本身(无子项)不算 asset:fail-closed。
        assert!(
            publisher
                .publish("aggregate_initialization_0001", ".aria/aggregate")
                .is_err()
        );
        // 非 aggregate 子树:fail-closed。
        assert!(
            publisher
                .publish("aggregate_initialization_0001", ".aria/other/config.json")
                .is_err()
        );
        assert!(publisher.published_paths().is_empty());
    }

    // ---- Task 17: profile 预检与 frontend pnpm/Vite 选择 ----

    /// 为 profile 预检构建一个真实的 logical codebase coordinator fixture:
    /// 在 temp 目录创建 aggregate root、member main checkout 目录,并持久化
    /// manifest + member + checkout。`member_root(name)` 返回该 member 的
    /// main checkout 根,使测试可以在里面写 package.json / vite.config.ts。
    struct ProfileFixture {
        _temp: tempfile::TempDir,
        _paths: ProductAppPaths,
        member_roots: std::collections::HashMap<String, PathBuf>,
        coordinator: AggregateInitializationCoordinator,
    }

    impl ProfileFixture {
        /// 返回指定别名 member 的 main checkout 根,供测试写入 profile 信号。
        fn member_root(&self, alias: &str) -> &Path {
            self.member_roots
                .get(alias)
                .unwrap_or_else(|| panic!("unknown member alias {alias}"))
        }

        fn preflight_profile(
            &self,
        ) -> Result<AggregateInitializationProfile, AggregateInitializationError> {
            self.coordinator.preflight_profile("project_0001")
        }

        fn preflight_commands(&self) -> Vec<String> {
            self.coordinator.preflight_commands("project_0001").unwrap()
        }
    }

    fn profile_fixture(member_aliases: &[&str]) -> ProfileFixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let store = AggregateInitializationOperationStore::new(paths.clone());

        let aggregate_root = temp.path().join("aggregate-root");
        std::fs::create_dir_all(&aggregate_root).unwrap();

        let mut member_roots = std::collections::HashMap::new();
        let mut member_ids = Vec::new();
        let lc_store = LogicalCodebaseStore::new(paths.clone());
        for (ordinal, alias) in member_aliases.iter().enumerate() {
            let member_dir = aggregate_root.join(alias);
            std::fs::create_dir_all(&member_dir).unwrap();
            let member_id = LogicalRepositoryId(Uuid::new_v4());
            let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
            member_ids.push(member_id);
            member_roots.insert((*alias).to_string(), member_dir.clone());
            let now = CREATED_AT.to_string();
            let member = CodebaseMemberRecord {
                logical_repository_id: member_id,
                physical_repository_id: format!("repository_{alias}"),
                alias: (*alias).to_string(),
                role: "service".to_string(),
                ordinal: ordinal as u32,
                source_identity: RepositorySourceIdentity::from_git_parts(
                    &member_dir,
                    member_dir.join(".git"),
                    Some(format!("ssh://git@example.test/acme/{alias}.git")),
                ),
                repo_type: Default::default(),
                tech_stack: Vec::new(),
                owner: None,
                tags: Vec::new(),
                default_ref: None,
                checkout_ids: vec![checkout_id],
                status: Default::default(),
                created_at: now.clone(),
                updated_at: now,
            };
            lc_store.save_member("project_0001", &member).unwrap();
            let now = CREATED_AT.to_string();
            let checkout = RepositoryCheckoutRecord {
                checkout_id,
                logical_repository_id: member_id,
                physical_repository_id: format!("repository_{alias}"),
                kind: crate::product::logical_codebase::CheckoutKind::Main,
                canonical_path: member_dir.clone(),
                checkout_path_hash: "sha256:checkout".to_string(),
                git_dir_identity: "sha256:git-dir".to_string(),
                revision: Some("abc123".to_string()),
                availability: crate::product::logical_codebase::CheckoutAvailability::Available,
                observed_at: now.clone(),
                created_at: now.clone(),
                updated_at: now,
            };
            lc_store.save_checkout("project_0001", &checkout).unwrap();
        }

        let manifest = LogicalCodebaseManifest::new("project_0001", aggregate_root, member_ids);
        lc_store.save_manifest("project_0001", &manifest).unwrap();

        let skills_calls = Arc::new(Mutex::new(Vec::new()));
        let preflight_calls = Arc::new(Mutex::new(Vec::new()));
        let skills: Arc<dyn AggregateSkillsPreparation> = Arc::new(FakeSkillsPreparation {
            calls: skills_calls,
        });
        let preflight: Arc<dyn AggregatePreflightService> = Arc::new(FakePreflightService {
            calls: preflight_calls,
        });
        let provider = Arc::new(FakeProviderTurnDriver::new());
        let clock: Arc<Clock> = Arc::new(|| CREATED_AT.to_string());
        let coordinator = AggregateInitializationCoordinator::new(
            paths.clone(),
            store,
            skills,
            preflight,
            provider,
            clock,
        );

        ProfileFixture {
            _temp: temp,
            _paths: paths.clone(),
            member_roots,
            coordinator,
        }
    }

    #[test]
    fn frontend_pnpm_vite_profile_changes_templates_not_stable_step_layout() {
        let fixture = profile_fixture(&["web"]);
        std::fs::write(
            fixture.member_root("web").join("package.json"),
            r#"{"packageManager":"pnpm@9","devDependencies":{"vite":"1"}}"#,
        )
        .unwrap();
        std::fs::write(
            fixture.member_root("web").join("pnpm-lock.yaml"),
            "lockfileVersion: '9.0'",
        )
        .unwrap();
        std::fs::write(
            fixture.member_root("web").join("vite.config.ts"),
            "export default {}",
        )
        .unwrap();

        let profile = fixture.preflight_profile().unwrap();
        assert_eq!(profile, AggregateInitializationProfile::FrontendPnpmVite);
        assert_eq!(AggregateInitializationStepKind::V1.len(), 5);
        assert!(
            !fixture
                .preflight_commands()
                .iter()
                .any(|command| command.contains("mvn") || command.contains("gradle"))
        );
    }

    #[test]
    fn java_backend_profile_resolves_when_all_members_are_backend() {
        let fixture = profile_fixture(&["api"]);
        std::fs::write(
            fixture.member_root("api").join("pom.xml"),
            "<project><modelVersion>4.0.0</modelVersion></project>",
        )
        .unwrap();

        let profile = fixture.preflight_profile().unwrap();
        assert_eq!(profile, AggregateInitializationProfile::JavaBackend);
        assert!(
            fixture
                .preflight_commands()
                .iter()
                .any(|command| command.contains("mvn"))
        );
    }

    #[test]
    fn mixed_profile_resolves_when_backend_and_frontend_members_coexist() {
        let fixture = profile_fixture(&["api", "web"]);
        std::fs::write(
            fixture.member_root("api").join("pom.xml"),
            "<project><modelVersion>4.0.0</modelVersion></project>",
        )
        .unwrap();
        std::fs::write(
            fixture.member_root("web").join("package.json"),
            r#"{"packageManager":"pnpm@9"}"#,
        )
        .unwrap();
        std::fs::write(
            fixture.member_root("web").join("pnpm-lock.yaml"),
            "lockfileVersion: '9.0'",
        )
        .unwrap();
        std::fs::write(
            fixture.member_root("web").join("vite.config.ts"),
            "export default {}",
        )
        .unwrap();

        let profile = fixture.preflight_profile().unwrap();
        assert_eq!(profile, AggregateInitializationProfile::Mixed);
    }

    #[test]
    fn unknown_profile_fails_preflight_closed() {
        let fixture = profile_fixture(&["stray"]);
        // No recognizable signals -> detect returns Unknown -> preflight fails closed.
        let error = fixture.preflight_profile().unwrap_err();
        assert!(matches!(
            error,
            AggregateInitializationError::Preflight { .. }
        ));
    }

    #[test]
    fn profile_preflight_commands_are_profile_specific_and_keep_five_step_layout() {
        // Frontend pnpm/Vite precheck never includes Maven/Gradle commands.
        let frontend = profile_preflight_commands(AggregateInitializationProfile::FrontendPnpmVite);
        assert!(frontend.iter().any(|command| command.contains("pnpm")));
        assert!(
            !frontend
                .iter()
                .any(|command| command.contains("mvn") || command.contains("gradle"))
        );

        // Java backend includes Maven.
        let java = profile_preflight_commands(AggregateInitializationProfile::JavaBackend);
        assert!(java.iter().any(|command| command.contains("mvn")));

        // Mixed composes both namespaced command sets.
        let mixed = profile_preflight_commands(AggregateInitializationProfile::Mixed);
        assert!(mixed.iter().any(|command| command.contains("mvn")));
        assert!(mixed.iter().any(|command| command.contains("pnpm")));

        // The five stable step IDs never change regardless of profile.
        assert_eq!(
            AggregateInitializationStepKind::V1.len(),
            5,
            "profile selection must not change the stable step layout"
        );
    }
}
