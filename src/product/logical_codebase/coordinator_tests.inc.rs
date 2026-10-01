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
            _lc_id: Option<&str>,
            _bootstrap: BootstrapPhaseCredential,
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
                _project_id: &str,
                _operation_id: &str,
                step: AggregateInitializationStepKind,
                _preflight: &AggregatePreflightSnapshot,
                _lc_id: Option<&str>,
                _bootstrap: BootstrapPhaseCredential,
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
                _project_id: &str,
                _operation_id: &str,
                _step: AggregateInitializationStepKind,
                _preflight: &AggregatePreflightSnapshot,
                _lc_id: Option<&str>,
                _bootstrap: BootstrapPhaseCredential,
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
            aggregate_root.clone(),
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
        // 隔离回归门:聚合初始化生产代码(非测试、非 doc comment)不得引用
        // 单仓持久化层、单仓 git 终结点或单仓六步 operation 机器,保证聚合
        // root recipe 不进入成员仓 git 调用图,也不复用 registration/GitFinalize
        // 外层(BOOT-01 后半句、D3)。Task 1.1 起扫描面从 coordinator `.inc.rs`
        // 扩大到聚合 operation 类型与 durable store;测试模块与 doc comment
        // 行被跳过以避免自指。单仓六步命令源 `RepositoryInitializationStepKind`
        // 不在禁用之列——root recipe 显式复用它的 `command()` 事实。
        let production_sources = [
            include_str!("aggregate_initialization_coordinator.rs"),
            include_str!("coordinator_lifecycle.inc.rs"),
            include_str!("coordinator_provider_turn.inc.rs"),
            include_str!("coordinator_preflight.inc.rs"),
            include_str!("coordinator_profile.inc.rs"),
            include_str!("aggregate_initialization.rs"),
            include_str!("aggregate_initialization_store.rs"),
        ];
        let forbidden = [
            "RepositoryPersistence",
            "git_finalize",
            "RepositoryRegistrationCoordinator",
            "RepositoryInitializationOperation",
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

        // 固定 Claude recipe 隔离门(REQ-BOOT-03「recipe provider SHALL 固定为
        // Claude Code」):聚合 turn 驱动生产面不得出现任何非 Claude provider
        // dialect 或按配置选择 provider 的工厂——那是新增 fallback 通道,
        // 本 change 明确不做。
        let forbidden_provider_channels = [
            "ProviderRef::codex",
            "from_provider_name",
            "ProviderRefType::Codex",
            "ProviderType::Codex",
            "ProviderType::Pi",
            "ProviderType::KimiCode",
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
                for token in forbidden_provider_channels {
                    assert!(
                        !line.contains(token),
                        "aggregate recipe is fixed to Claude Code and must not add provider fallback {token}: {line}"
                    );
                }
            }
        }

        // 迁移边界说明(Task 1.1):真实四命令/超时契约不在本锁内断言——当前
        // `pre_check` 等 turn 的 streaming_input 仍是占位 prompt 与 1s 占位
        // 超时(本任务 Step 2 已用红证记录该缺口:prompt 为
        // "aggregate initialization turn: pre_check" 而非真实命令)。真实四命令
        // 行为锁由 lc-root-initialization Task 1.4 的
        // `lc_claude_recipe_runs_four_commands_once_in_fixed_root_order` 交付;
        // 本测试只固定迁移边界:不新增 step、不进单仓 registration/GitFinalize、
        // recipe 固定 Claude Code(上方扫描门)。
    }

    /// Task 1.1 迁移边界回归锁:LC root-cwd 契约(BOOT-01/REQ-BOOT-03)下的
    /// 聚合根 recipe 隔离事实——五步布局零变化(trust 前置门/四命令不得成为
    /// 第六步)、三个 provider turn 唯一经 gateway 启动且全部为固定 Claude
    /// Code、每个 turn 的 cwd 都是 canonical 聚合根(绝不为成员 checkout 根)。
    /// 供后续 recipe/receipt 任务消费:任何复用单仓 registration/GitFinalize、
    /// 新增 provider fallback 或把 turn cwd 挪进成员仓的改动都会在此先红。
    #[tokio::test]
    async fn aggregate_root_contract_does_not_reuse_repository_registration_or_git_finalize() {
        let fixture = gateway_aggregate_fixture();
        let operation = fixture
            .coordinator()
            .execute(
                "project_0001",
                "aggregate_initialization_0001",
                CancellationToken::new(),
            )
            .await
            .unwrap();

        // 五步布局零变化:步骤 id 顺序 == V1 且全部 Completed;operation kind
        // 仍是聚合专属判别码,不与单仓六步 operation 混流。
        assert_eq!(
            operation
                .steps
                .iter()
                .map(|step| step.step_id)
                .collect::<Vec<_>>(),
            AggregateInitializationStepKind::V1.to_vec(),
            "root recipe must map onto the existing five steps; no sixth step may appear"
        );
        assert!(operation
            .steps
            .iter()
            .all(|step| step.status.as_str() == "completed"));
        assert_eq!(operation.operation_kind, "aggregate_initialization");

        // 三个 provider turn 唯一经 gateway 流式启动,无同步栈旁路,且每次
        // 启动都携带 policy digest——聚合根路径上不存在不经 gateway 的单仓
        // registration/GitFinalize 式 provider 启动。
        assert_eq!(fixture.gateway_audit().stream_launches(), 3);
        assert_eq!(fixture.gateway_audit().sync_launches(), 0);
        assert!(fixture.gateway_audit().all_have_policy_digest());

        // 固定 Claude recipe:每次启动的 provider_type 都是 ClaudeCode,
        // 不存在 Codex/Pi/KimiCode fallback 通道。
        let inputs = fixture.streaming_inputs();
        assert_eq!(inputs.len(), 3);
        assert!(
            inputs
                .iter()
                .all(|input| input.provider_type
                    == crate::protocol::contracts::ProviderType::ClaudeCode),
            "aggregate recipe provider must stay fixed to Claude Code"
        );

        // root-cwd 契约:每个 provider turn 的 cwd 都是 canonical 聚合根本身,
        // 绝不是任何成员 checkout 根(单仓 cwd 契约不被聚合复用)。
        let canonical_root = std::fs::canonicalize(fixture.aggregate_root()).unwrap();
        assert!(
            inputs.iter().all(|input| input.working_dir == canonical_root),
            "every provider turn must run with cwd = canonical aggregate root"
        );
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

        let manifest =
            LogicalCodebaseManifest::new("project_0001", aggregate_root.clone(), member_ids);
        lc_store.save_manifest("project_0001", &manifest).unwrap();

        let skills_calls = Arc::new(Mutex::new(Vec::new()));
        let preflight_calls = Arc::new(Mutex::new(Vec::new()));
        let skills: Arc<dyn AggregateSkillsPreparation> = Arc::new(FakeSkillsPreparation {
            calls: skills_calls,
        });
        let preflight: Arc<dyn AggregatePreflightService> = Arc::new(FakePreflightService::new(
            preflight_calls,
            aggregate_root.to_string_lossy().into_owned(),
        ));
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

    // ---- C4 Task 4：member-index checkpoint 中断可重入，不重复已完成 provider turn ----

    #[tokio::test]
    async fn member_index_checkpoint_survives_interruption_and_does_not_repeat_completed_provider_turn()
     {
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

        /// 第一个 provider turn 完成后触发 cancellation，模拟页面关闭/进程中断。
        struct InterruptAfterFirstTurnProvider {
            turns: Mutex<u32>,
            token: CancellationToken,
        }
        #[async_trait]
        impl AggregateProviderTurnDriver for InterruptAfterFirstTurnProvider {
            async fn run_turn(
                &self,
                _project_id: &str,
                _operation_id: &str,
                _step: AggregateInitializationStepKind,
                _preflight: &AggregatePreflightSnapshot,
                _lc_id: Option<&str>,
                _bootstrap: BootstrapPhaseCredential,
                _cancellation: CancellationToken,
            ) -> Result<String, AggregateInitializationError> {
                let mut turns = self.turns.lock().unwrap();
                *turns += 1;
                if *turns == 1 {
                    self.token.cancel();
                }
                Ok("summary".to_string())
            }
        }

        let skills: Arc<dyn AggregateSkillsPreparation> = Arc::new(FakeSkillsPreparation {
            calls: Arc::new(Mutex::new(Vec::new())),
        });
        let preflight = Arc::new(FakePreflightService::new(
            Arc::new(Mutex::new(Vec::new())),
            manifest.provider_context_root.to_string_lossy().into_owned(),
        ));
        let preflight_driver: Arc<dyn AggregatePreflightService> = preflight.clone();
        let token = CancellationToken::new();
        let provider = Arc::new(InterruptAfterFirstTurnProvider {
            turns: Mutex::new(0),
            token: token.clone(),
        });
        let clock: Arc<Clock> = Arc::new(|| CREATED_AT.to_string());
        let coordinator = AggregateInitializationCoordinator::new(
            paths.clone(),
            store.clone(),
            skills.clone(),
            preflight_driver.clone(),
            provider.clone(),
            clock,
        );
        coordinator
            .begin(
                "aggregate_initialization_c4".to_string(),
                "project_0001",
                AggregateInitializationOperationInput {
                    idempotency_key: "c4-resume".to_string(),
                    manifest_revision: manifest.membership_revision,
                    policy_digest: "sha256:policy".to_string(),
                    profile_evidence_digest: Some("sha256:profile".to_string()),
                    provider_context_root: manifest.provider_context_root.clone(),
                    provider: "claude_code".to_string(),
                },
            )
            .unwrap();

        let interrupted = coordinator
            .execute("project_0001", "aggregate_initialization_c4", token)
            .await;
        assert!(matches!(
            interrupted,
            Err(AggregateInitializationError::Cancelled)
        ));

        // 中断现场：deterministic member-index（preflight）checkpoint 保留，
        // 已完成的 provider turn 不重跑。
        let operation = coordinator
            .get("project_0001", "aggregate_initialization_c4")
            .unwrap();
        assert_eq!(
            operation.status,
            AggregateInitializationOperationStatus::Failed
        );
        let preflight_step = operation
            .steps
            .iter()
            .find(|step| step.step_id == AggregateInitializationStepKind::AggregatePreflight)
            .unwrap();
        assert_eq!(
            preflight_step.status,
            AggregateInitializationStepStatus::Completed
        );
        assert!(preflight_step.output_artifact_ref.is_some());
        assert_eq!(*provider.turns.lock().unwrap(), 1);

        // 显式 Continue：execute_remaining 只执行未完成步骤。
        let preflight_calls_before = preflight.calls.lock().unwrap().len();
        let completed = coordinator
            .execute_remaining(
                "project_0001",
                "aggregate_initialization_c4",
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(
            completed.status,
            AggregateInitializationOperationStatus::Completed
        );
        // provider turn 计数 = 3（首次 1 + 续跑 2），已完成的 turn 不重复。
        assert_eq!(*provider.turns.lock().unwrap(), 3);
        // deterministic preflight 不重跑（Completed 步骤跳过）。
        assert_eq!(preflight.calls.lock().unwrap().len(), preflight_calls_before);
    }

    // ---- Task 1.4（REQ-REG-14/REQ-BOOT-03/BOOT-04）：trust 硬前置门 + 真实四命令 root recipe ----

    use crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential;
    use crate::product::logical_codebase::provider_trust::{
        ProviderTrustPrecondition, ProviderTrustPreparationResult, ProviderTrustWaiting,
    };

    /// 可切换 trust gate fake：`fail` 为真时返回稳定 reason_code 的可重试
    /// Waiting；翻绿后同一调用面返回 Ready。
    struct SwitchableTrustGate {
        fail: std::sync::atomic::AtomicBool,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl SwitchableTrustGate {
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

    impl ProviderTrustPrecondition for SwitchableTrustGate {
        fn ensure_before_recipe(
            &self,
            _project_id: &str,
            _operation_id: &str,
            _lc_id: &str,
            canonical_root: &Path,
            _providers: &[ProviderName],
        ) -> ProviderTrustPreparationResult {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
                return ProviderTrustPreparationResult::Waiting {
                    waiting: ProviderTrustWaiting {
                        provider: ProviderName::KimiCode,
                        canonical_root: canonical_root.to_path_buf(),
                        trust_key: "wd_test_root_abc123def456".to_string(),
                        reason_code: "kimi_trust_blocked_for_test".to_string(),
                        message: "workspace trust entry is blocked for test".to_string(),
                        retry_action:
                            "Resolve the blocked workspace trust entry, then retry aggregate initialization."
                                .to_string(),
                        recorded_at: CREATED_AT.to_string(),
                    },
                };
            }
            ProviderTrustPreparationResult::Ready {
                registrations: Vec::new(),
            }
        }
    }

    /// trust 门 + FakeProviderTurnDriver 的 coordinator 级 fixture：preflight
    /// snapshot 根与 manifest/operation 的 provider_context_root 同源，使
    /// bootstrap phase credential 可从 durable Running operation 派生。
    struct TrustRecipeFixture {
        _temp: tempfile::TempDir,
        skills_calls: Arc<Mutex<Vec<String>>>,
        provider: Arc<FakeProviderTurnDriver>,
        gate: Arc<SwitchableTrustGate>,
        store: AggregateInitializationOperationStore,
        coordinator: AggregateInitializationCoordinator,
        input: AggregateInitializationOperationInput,
    }

    fn trust_recipe_fixture(gate_fail: bool) -> TrustRecipeFixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let store = AggregateInitializationOperationStore::new(paths.clone());
        let aggregate_root = temp.path().join("aggregate-root");
        std::fs::create_dir_all(&aggregate_root).unwrap();
        let manifest =
            LogicalCodebaseManifest::new("project_0001", aggregate_root.clone(), Vec::new());
        LogicalCodebaseStore::new(paths.clone())
            .save_manifest("project_0001", &manifest)
            .unwrap();

        let skills_calls = Arc::new(Mutex::new(Vec::new()));
        let skills: Arc<dyn AggregateSkillsPreparation> = Arc::new(FakeSkillsPreparation {
            calls: skills_calls.clone(),
        });
        let preflight: Arc<dyn AggregatePreflightService> = Arc::new(FakePreflightService::new(
            Arc::new(Mutex::new(Vec::new())),
            aggregate_root.to_string_lossy().into_owned(),
        ));
        let provider = Arc::new(FakeProviderTurnDriver::new());
        let gate = Arc::new(SwitchableTrustGate::new(gate_fail));
        let clock: Arc<Clock> = Arc::new(|| CREATED_AT.to_string());
        let coordinator = AggregateInitializationCoordinator::new(
            paths.clone(),
            store.clone(),
            skills,
            preflight,
            provider.clone(),
            clock,
        )
        .with_trust(gate.clone());

        let input = AggregateInitializationOperationInput {
            idempotency_key: "trust-0001".to_string(),
            manifest_revision: manifest.membership_revision,
            policy_digest: "sha256:policy".to_string(),
            profile_evidence_digest: Some("sha256:profile".to_string()),
            provider_context_root: manifest.provider_context_root.clone(),
            provider: "claude_code".to_string(),
        };
        TrustRecipeFixture {
            _temp: temp,
            skills_calls,
            provider,
            gate,
            store,
            coordinator,
            input,
        }
    }

    #[tokio::test]
    async fn trust_failure_blocks_recipe_and_keeps_operation_uncreated() {
        let fixture = trust_recipe_fixture(true);
        let result = fixture
            .coordinator
            .execute_with_trust(
                "aggregate_initialization_trust_0001".to_string(),
                "project_0001",
                fixture.input.clone(),
                &[ProviderName::Codex, ProviderName::KimiCode],
                CancellationToken::new(),
            )
            .await;

        // 任一 trust 未 Ready：可重试 waiting 面（稳定 reason_code + 重试动作）。
        let Err(AggregateInitializationError::TrustWaiting { waiting }) = result else {
            panic!("trust failure must surface a retryable waiting error before any recipe start");
        };
        assert_eq!(waiting.reason_code, "kimi_trust_blocked_for_test");
        assert!(!waiting.retry_action.is_empty());

        // 五步 operation 不创建：coordinator 与 durable store 双面均 NotFound。
        assert!(matches!(
            fixture
                .coordinator
                .get("project_0001", "aggregate_initialization_trust_0001"),
            Err(AggregateInitializationError::NotFound { .. })
        ));
        assert!(fixture
            .store
            .get("project_0001", "aggregate_initialization_trust_0001")
            .is_err());

        // provider 启动计数为 0，确定性 step 也未运行。
        assert_eq!(fixture.provider.turn_count(), 0);
        assert!(fixture.skills_calls.lock().unwrap().is_empty());
        assert_eq!(fixture.gate.calls(), 1);
    }

    #[tokio::test]
    async fn trust_retry_success_starts_claude_recipe() {
        let fixture = trust_recipe_fixture(true);
        let first = fixture
            .coordinator
            .execute_with_trust(
                "aggregate_initialization_trust_0001".to_string(),
                "project_0001",
                fixture.input.clone(),
                &[ProviderName::Codex, ProviderName::KimiCode],
                CancellationToken::new(),
            )
            .await;
        assert!(matches!(
            first,
            Err(AggregateInitializationError::TrustWaiting { .. })
        ));
        assert_eq!(fixture.provider.turn_count(), 0);

        // 等待面可重试：同一 operation id 再来一次，gate Ready 后五步 recipe
        // 按固定顺序启动并完成。
        fixture.gate.set_ready();
        let operation = fixture
            .coordinator
            .execute_with_trust(
                "aggregate_initialization_trust_0001".to_string(),
                "project_0001",
                fixture.input.clone(),
                &[ProviderName::Codex, ProviderName::KimiCode],
                CancellationToken::new(),
            )
            .await
            .expect("retry after trust readiness must start the five-step recipe");
        assert_eq!(
            operation.status,
            AggregateInitializationOperationStatus::Completed
        );
        assert_eq!(
            operation
                .steps
                .iter()
                .map(|step| step.step_id)
                .collect::<Vec<_>>(),
            AggregateInitializationStepKind::V1.to_vec()
        );
        assert_eq!(fixture.provider.turn_count(), 3);
        assert_eq!(fixture.gate.calls(), 2);
    }

    #[tokio::test]
    async fn lc_claude_recipe_runs_four_commands_once_in_fixed_root_order() {
        let fixture = gateway_aggregate_fixture();
        let operation = fixture
            .coordinator()
            .execute_with_trust(
                "aggregate_initialization_0001".to_string(),
                "project_0001",
                fixture.recipe_input(),
                &[ProviderName::ClaudeCode],
                CancellationToken::new(),
            )
            .await
            .expect("claude-only recipe must pass the trust gate and complete");
        assert_eq!(
            operation.status,
            AggregateInitializationOperationStatus::Completed
        );

        let inputs = fixture.streaming_inputs();
        assert_eq!(inputs.len(), 3);

        // 四命令文本唯一来源：RepositoryInitializationStepKind::command()。
        use crate::product::repository_store::RepositoryInitializationStepKind as RepoStep;
        let pre_check = RepoStep::PreCheck.command().unwrap();
        let rule_config = RepoStep::RuleConfig.command().unwrap();
        let mcp_config = RepoStep::McpConfiguration.command().unwrap();
        let rules_examples = RepoStep::ProjectRulesExamples.command().unwrap();

        // 固定顺序：PreCheck=命令1；RuleAndMcpConfig=命令2+3（同一 Claude
        // turn）；OpenspecAndExamples=命令4。
        assert_eq!(inputs[0].prompt, pre_check);
        assert_eq!(inputs[1].prompt, format!("{rule_config}\n{mcp_config}"));
        assert_eq!(inputs[2].prompt, rules_examples);

        // 每条命令在根上恰好执行一次。
        let joined = inputs
            .iter()
            .map(|input| input.prompt.clone())
            .collect::<Vec<_>>()
            .join("\n");
        for command in [pre_check, rule_config, mcp_config, rules_examples] {
            assert_eq!(
                joined.matches(command).count(),
                1,
                "root recipe command must run exactly once: {command}"
            );
        }

        // 固定 Claude Code、真实命令超时（非 1s 占位）、canonical 聚合根 cwd。
        assert!(
            inputs.iter().all(|input| input.provider_type
                == crate::protocol::contracts::ProviderType::ClaudeCode),
            "aggregate recipe provider must stay fixed to Claude Code"
        );
        assert_eq!(
            inputs[0].timeout_secs,
            GatewayBackedAggregateProviderTurnDriver::DEFAULT_COMMAND_TIMEOUT_SECS
        );
        let canonical_root = std::fs::canonicalize(fixture.aggregate_root()).unwrap();
        assert!(
            inputs.iter().all(|input| input.working_dir == canonical_root),
            "every root recipe command must run with cwd = canonical aggregate root"
        );
        assert_eq!(fixture.streaming_start_count(), 3);
        assert_eq!(fixture.gateway_audit().stream_launches(), 3);
    }

    /// 失败 step 之后的全部步骤保持 Pending（REQ-BOOT-03「任一命令失败停止
    /// 后续命令并保留失败事实」）。
    fn assert_later_steps_pending(
        operation: &AggregateInitializationOperation,
        failed: AggregateInitializationStepKind,
    ) {
        let mut after_failed = false;
        for step in &operation.steps {
            if step.step_id == failed {
                after_failed = true;
                continue;
            }
            if after_failed {
                assert_eq!(
                    step.status.as_str(),
                    "pending",
                    "step {} must stay pending after {:?} failed",
                    step.step_id.as_str(),
                    failed
                );
            }
        }
    }

    #[tokio::test]
    async fn recipe_cancellation_timeout_and_failure_leave_durable_facts() {
        // a) provider 报告失败：首 turn 失败 → operation Failed，后续命令 Pending。
        let failure = gateway_aggregate_fixture_with(StreamingBehavior::Fail, None);
        let result = failure
            .coordinator()
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
        let operation = failure
            .coordinator()
            .get("project_0001", "aggregate_initialization_0001")
            .unwrap();
        assert_eq!(
            operation.status,
            AggregateInitializationOperationStatus::Failed
        );
        assert_eq!(
            operation.failed_step,
            Some(AggregateInitializationStepKind::PreCheck)
        );
        assert_eq!(failure.streaming_start_count(), 1);
        assert_later_steps_pending(&operation, AggregateInitializationStepKind::PreCheck);

        // b) 命令超时：真实 per-command 超时（50ms）内无完成事件 → 失败事实
        //    durable 保留，后续命令 Pending。
        let timeout = gateway_aggregate_fixture_with(
            StreamingBehavior::Hang,
            Some(std::time::Duration::from_millis(50)),
        );
        let result = timeout
            .coordinator()
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
        let operation = timeout
            .coordinator()
            .get("project_0001", "aggregate_initialization_0001")
            .unwrap();
        assert_eq!(
            operation.status,
            AggregateInitializationOperationStatus::Failed
        );
        assert_eq!(
            operation.failed_step,
            Some(AggregateInitializationStepKind::PreCheck)
        );
        assert_eq!(timeout.streaming_start_count(), 1);
        assert_later_steps_pending(&operation, AggregateInitializationStepKind::PreCheck);

        // c) turn 挂起中取消：durable 失败事实保留，后续命令 Pending。
        let cancelled = gateway_aggregate_fixture_with(StreamingBehavior::CancelWhileHanging, None);
        let result = cancelled
            .coordinator()
            .execute(
                "project_0001",
                "aggregate_initialization_0001",
                CancellationToken::new(),
            )
            .await;
        assert!(result.is_err());
        let operation = cancelled
            .coordinator()
            .get("project_0001", "aggregate_initialization_0001")
            .unwrap();
        assert!(matches!(
            operation.status,
            AggregateInitializationOperationStatus::Failed
                | AggregateInitializationOperationStatus::Cancelled
        ));
        assert_eq!(cancelled.streaming_start_count(), 1);
        assert_later_steps_pending(&operation, AggregateInitializationStepKind::PreCheck);
    }
}
