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
            request: AggregateProviderTurnRequest<'_>,
        ) -> Result<String, AggregateInitializationError> {
            let AggregateProviderTurnRequest { step, .. } = request;
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
        /// Task 1.4：snapshot 的聚合根必须与 manifest/operation 的
        /// provider_context_root 同源——bootstrap phase credential 派生会
        /// 比对该根，占位 "/aggregate-root" 会被判 root 漂移。
        aggregate_root: String,
    }

    impl FakePreflightService {
        fn new(calls: Arc<Mutex<Vec<String>>>, aggregate_root: impl Into<String>) -> Self {
            Self {
                calls,
                aggregate_root: aggregate_root.into(),
            }
        }
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
                aggregate_root: self.aggregate_root.clone(),
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
        let preflight: Arc<dyn AggregatePreflightService> = Arc::new(FakePreflightService::new(
            preflight_calls.clone(),
            manifest.provider_context_root.to_string_lossy().into_owned(),
        ));
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
                request: AggregateProviderTurnRequest<'_>,
            ) -> Result<String, AggregateInitializationError> {
                let AggregateProviderTurnRequest { step, .. } = request;
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
        let preflight: Arc<dyn AggregatePreflightService> = Arc::new(FakePreflightService::new(
            Arc::new(Mutex::new(Vec::new())),
            manifest.provider_context_root.to_string_lossy().into_owned(),
        ));
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
                _request: AggregateProviderTurnRequest<'_>,
            ) -> Result<String, AggregateInitializationError> {
                let mut count = self.count.lock().unwrap();
                *count += 1;
                Ok("summary".to_string())
            }
        }

        let skills: Arc<dyn AggregateSkillsPreparation> = Arc::new(FakeSkillsPreparation {
            calls: Arc::new(Mutex::new(Vec::new())),
        });
        let preflight: Arc<dyn AggregatePreflightService> = Arc::new(FakePreflightService::new(
            Arc::new(Mutex::new(Vec::new())),
            manifest.provider_context_root.to_string_lossy().into_owned(),
        ));
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

    /// 测试用 streaming adapter 行为开关（Task 1.4）：
    /// - `Complete`：立即发一条 Completed 事件（既有默认，保持原测试语义）；
    /// - `Hang`：事件流保持打开且永不发事件——驱动 select 只能靠超时臂收口；
    /// - `Fail`：立即发 Failed 事件——驱动按 provider 报告失败收口；
    /// - `CancelWhileHanging`：先取消共享 token 再保持挂起——驱动取消臂确定性触发。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum StreamingBehavior {
        Complete,
        Hang,
        Fail,
        CancelWhileHanging,
    }

    /// 测试用 streaming adapter:记录 start 调用次数并立即完成会话。
    /// Task 1.1 seam:同时按启动顺序捕获 `StreamingProviderInput` 快照,
    /// 供「固定 Claude recipe / root-cwd」隔离断言消费;计数语义不变。
    /// (parking_lot 非本 crate 依赖,沿用本文件 std Mutex 约定。)
    struct CountingStreamingAdapter {
        behavior: StreamingBehavior,
        start_count: std::sync::atomic::AtomicUsize,
        inputs: Mutex<Vec<crate::cross_cutting::streaming_provider::StreamingProviderInput>>,
    }

    impl CountingStreamingAdapter {
        fn new(behavior: StreamingBehavior) -> Self {
            Self {
                behavior,
                start_count: std::sync::atomic::AtomicUsize::new(0),
                inputs: Mutex::new(Vec::new()),
            }
        }

        fn start_count(&self) -> usize {
            self.start_count.load(std::sync::atomic::Ordering::SeqCst)
        }

        /// 每次成功 start 的输入快照,按启动顺序返回。
        fn started_inputs(
            &self,
        ) -> Vec<crate::cross_cutting::streaming_provider::StreamingProviderInput> {
            self.inputs.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl crate::cross_cutting::streaming_provider::StreamingProviderAdapter
        for CountingStreamingAdapter
    {
        async fn start(
            &self,
            input: crate::cross_cutting::streaming_provider::StreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<
            crate::cross_cutting::streaming_provider::ProviderSession,
            crate::cross_cutting::provider_adapter::ProviderAdapterError,
        > {
            self.start_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inputs.lock().unwrap().push(input);
            let (event_tx, events) = tokio::sync::mpsc::channel(1);
            let (commands, _command_rx) = tokio::sync::mpsc::channel(1);
            match self.behavior {
                StreamingBehavior::Complete => {
                    let _ = event_tx.try_send(
                        crate::cross_cutting::streaming_provider::ProviderEvent::Completed(
                            crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                                "aggregate turn complete",
                                None,
                            ),
                        ),
                    );
                }
                StreamingBehavior::Fail => {
                    let _ = event_tx.try_send(
                        crate::cross_cutting::streaming_provider::ProviderEvent::Failed {
                            message: "simulated aggregate recipe failure".to_string(),
                        },
                    );
                }
                StreamingBehavior::CancelWhileHanging => {
                    _cancel.cancel();
                    // 事件流保持打开：取消臂必须是唯一可决议的 select 分支。
                    std::mem::forget(event_tx);
                }
                StreamingBehavior::Hang => {
                    // 事件流保持打开且永不完成：超时臂是唯一出口。
                    std::mem::forget(event_tx);
                }
            }
            // 挂起行为依赖事件流保持打开：不关闭接收端。
            Ok(crate::cross_cutting::streaming_provider::ProviderSession { events, commands, native_session_id: None })
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
        aggregate_root: PathBuf,
        audit: Arc<GatewayRunAudit>,
        streaming_adapter: Arc<CountingStreamingAdapter>,
        coordinator: AggregateInitializationCoordinator,
        /// Task 1.4：与 fixture 内已 begin 的 operation 完全同形的 input，
        /// 供 `execute_with_trust` 幂等重放（begin 命中既有 operation）。
        recipe_input: AggregateInitializationOperationInput,
    }

    impl GatewayAggregateFixture {
        fn coordinator(&self) -> &AggregateInitializationCoordinator {
            &self.coordinator
        }

        /// Task 1.1 seam:聚合根路径,供 root-cwd 断言 canonicalize 比对。
        fn aggregate_root(&self) -> PathBuf {
            self.aggregate_root.clone()
        }

        fn gateway_audit(&self) -> Arc<GatewayRunAudit> {
            self.audit.clone()
        }

        fn streaming_start_count(&self) -> usize {
            self.streaming_adapter.start_count()
        }

        /// Task 1.1 seam:按启动顺序返回 provider turn 的流式输入快照。
        fn streaming_inputs(
            &self,
        ) -> Vec<crate::cross_cutting::streaming_provider::StreamingProviderInput> {
            self.streaming_adapter.started_inputs()
        }

        /// Task 1.4：与 fixture 已 begin 的 operation 同形的 recipe input。
        fn recipe_input(&self) -> AggregateInitializationOperationInput {
            self.recipe_input.clone()
        }
    }

    fn gateway_aggregate_fixture() -> GatewayAggregateFixture {
        gateway_aggregate_fixture_with(StreamingBehavior::Complete, None)
    }

    /// Task 1.4：可参数化的 gateway fixture——streaming 行为与 provider turn
    /// 命令超时可注入，供取消/超时/失败 durable 事实测试消费；trust 门使用
    /// 真实 `HomeBackedProviderTrustRegistry`（fake home 目录，Claude-only
    /// recipe 下 gate 无需登记即 Ready）。
    fn gateway_aggregate_fixture_with(
        behavior: StreamingBehavior,
        command_timeout: Option<std::time::Duration>,
    ) -> GatewayAggregateFixture {
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
        let streaming_adapter = Arc::new(CountingStreamingAdapter::new(behavior));
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
            aggregate_root.clone(),
        ));

        let skills: Arc<dyn AggregateSkillsPreparation> = Arc::new(FakeSkillsPreparation {
            calls: Arc::new(Mutex::new(Vec::new())),
        });
        // 用真实 `DeterministicAggregatePreflightService` 以产出可 canonicalize 的聚合根快照,
        // 使 gateway spawn 前 cwd 复验能通过。
        let preflight: Arc<dyn AggregatePreflightService> =
            Arc::new(DeterministicAggregatePreflightService::new(paths.clone()));
        let driver = GatewayBackedAggregateProviderTurnDriver::claude_code(
            gateway,
            "cap_claude_code_1_4_0",
        );
        let provider: Arc<dyn AggregateProviderTurnDriver> = Arc::new(match command_timeout {
            Some(timeout) => driver.with_command_timeout(timeout),
            None => driver,
        });
        let clock: Arc<Clock> = Arc::new(|| CREATED_AT.to_string());
        let trust_home = temp.path().join("trust-home");
        std::fs::create_dir_all(&trust_home).unwrap();
        let trust_registry = crate::product::logical_codebase::HomeBackedProviderTrustRegistry::new(
            paths.clone(),
            vec![
                std::sync::Arc::new(
                    crate::product::logical_codebase::provider_trust_adapters::CodexTrustAdapter::for_home(
                        &trust_home,
                    ),
                ),
                std::sync::Arc::new(
                    crate::product::logical_codebase::provider_trust_adapters::KimiTrustAdapter::for_home(
                        &trust_home,
                    ),
                ),
            ],
        );
        let coordinator = AggregateInitializationCoordinator::new(
            paths.clone(),
            store.clone(),
            skills,
            preflight,
            provider,
            clock,
        )
        .with_trust(Arc::new(trust_registry));

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
                input.clone(),
            )
            .unwrap();

        GatewayAggregateFixture {
            _temp: temp,
            aggregate_root,
            audit,
            streaming_adapter,
            coordinator,
            recipe_input: input,
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

    // 以下测试段按主题拆入子 include,共享 mod tests 作用域
    // (large_file_guard 的 1200 行上限,纯移动,不改变任何行为)。
    include!("coordinator_tests_contract.inc.rs");
    include!("coordinator_tests_profile_trust.inc.rs");
}
