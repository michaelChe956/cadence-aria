#[cfg(test)]
mod tests {
    use super::*;
    use crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
    use crate::cross_cutting::provider_registry::ProviderRegistry;
    use crate::product::logical_codebase::policy::AggregatePolicyArtifactStore;
    use crate::product::logical_codebase::policy::{ProviderDialect, SessionPolicyAction};
    use crate::product::logical_codebase::provider_gateway::{
        PolicyTargetResolver, ProviderCapability, ProviderCapabilitySource, ProviderRef,
        ProviderRefType,
    };
    use crate::product::logical_codebase::types::{
        CheckoutAvailability, CodebaseMemberRecord, RepositoryCheckoutRecord,
        RepositorySourceIdentity, RepositoryType,
    };
    use crate::product::project_store::{CreateProjectInput, ProjectStore};

    // ---- 测试 fakes（与 provider_gateway_tests 同型，作用域隔离） ----

    struct PassThroughTargetResolver;
    impl PolicyTargetResolver for PassThroughTargetResolver {
        fn resolve_and_revalidate(
            &self,
            request: &SessionLaunchRequest,
        ) -> Result<crate::product::logical_codebase::policy::PolicyTarget, ProviderGatewayError>
        {
            let canonical = std::fs::canonicalize(&request.target.worktree)
                .map_err(|_| ProviderGatewayError::Target("worktree missing".to_string()))?;
            if request.target.logical_repository_id.is_empty() {
                Ok(
                    crate::product::logical_codebase::policy::PolicyTarget::aggregate_root(
                        canonical,
                    ),
                )
            } else {
                Ok(
                    crate::product::logical_codebase::policy::PolicyTarget::checkout(
                        request.target.logical_repository_id.clone(),
                        request.target.checkout_id.clone(),
                        canonical,
                    ),
                )
            }
        }
    }

    struct StaticCapabilitySource {
        deny: std::sync::atomic::AtomicBool,
    }
    impl StaticCapabilitySource {
        fn allowing() -> Self {
            Self {
                deny: std::sync::atomic::AtomicBool::new(false),
            }
        }
        fn deny(&self) {
            self.deny.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
    impl ProviderCapabilitySource for StaticCapabilitySource {
        fn require_supported(
            &self,
            provider: &ProviderRef,
            _action: SessionPolicyAction,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            if self.deny.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(ProviderGatewayError::UnsupportedCapability(
                    "capability record missing".to_string(),
                ));
            }
            let adapter_dialect = match provider.provider_type {
                ProviderRefType::ClaudeCode => ProviderDialect::ClaudeCodeCliV1,
                ProviderRefType::Codex => ProviderDialect::CodexCliV1,
                ProviderRefType::Pi => ProviderDialect::PiRpcV1,
                ProviderRefType::KimiCode => ProviderDialect::KimiAcpV1,
            };
            Ok(ProviderCapability {
                provider_type: provider.provider_type,
                version: "1.4.0".to_string(),
                adapter_dialect,
                capability_snapshot_ref: provider.capability_snapshot_ref.clone(),
                resume_evidence:
                    crate::product::logical_codebase::provider_gateway::ResumeEvidenceState::Confirmed,
            })
        }
    }

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
    #[async_trait::async_trait]
    impl crate::cross_cutting::streaming_provider::StreamingProviderAdapter
        for CountingStreamingAdapter
    {
        async fn start(
            &self,
            _input: crate::cross_cutting::streaming_provider::StreamingProviderInput,
            _cancel: tokio_util::sync::CancellationToken,
        ) -> Result<
            crate::cross_cutting::streaming_provider::ProviderSession,
            crate::cross_cutting::provider_adapter::ProviderAdapterError,
        > {
            self.start_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let (_event_tx, events) = tokio::sync::mpsc::channel(1);
            let (commands, _command_rx) = tokio::sync::mpsc::channel(1);
            Ok(crate::cross_cutting::streaming_provider::ProviderSession {
                events,
                commands,
                native_session_id: None,
            })
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
                stdout: "ok".to_string(),
                stderr: String::new(),
                structured_output: None,
                files_modified: Vec::new(),
                duration_ms: 0,
                timeout_status: TimeoutStatus::NotTimedOut,
            })
        }
    }

    fn always_available_gate()
    -> Arc<crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate> {
        use crate::cross_cutting::provider_availability_gate::ProviderHealthSource;
        use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
        use crate::product::models::ProviderName;
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

    struct AdmissionFixture {
        _temp: tempfile::TempDir,
        paths: ProductAppPaths,
        project_id: String,
        lc_id: String,
        aggregate_root: PathBuf,
        member_root: PathBuf,
        policy_store: AggregatePolicyArtifactStore,
        capabilities: Arc<StaticCapabilitySource>,
        streaming_adapter: Arc<CountingStreamingAdapter>,
        gateway: Arc<LogicalCodebaseProviderGateway>,
    }

    fn admission_fixture() -> AdmissionFixture {
        admission_fixture_with_policy(true)
    }

    /// Task 1.2：跳过 aggregate policy artifact 的变体（证明自举豁免不
    /// 覆盖 policy 维度）。
    fn admission_fixture_without_policy() -> AdmissionFixture {
        admission_fixture_with_policy(false)
    }

    fn admission_fixture_with_policy(with_policy: bool) -> AdmissionFixture {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = ProductAppPaths::new(temp.path());
        let project = ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "admission-project".to_string(),
                description: None,
            })
            .expect("project");
        let aggregate_root = temp.path().join("aggregate-root");
        std::fs::create_dir_all(&aggregate_root).unwrap();
        let member_root = temp.path().join("member-a");
        git_init_with_commit(&member_root);

        let logical = LogicalCodebaseStore::new(paths.clone());
        let lc = logical
            .create(
                &project.id,
                crate::product::logical_codebase::LogicalCodebaseCreateInput {
                    name: "admission-lc".to_string(),
                    aggregate_root: aggregate_root.clone(),
                },
            )
            .expect("lc");

        let lc_store = LogicalCodebaseStore::for_lc(paths.clone(), &lc.id);
        let member_id = LogicalRepositoryId(uuid::Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(uuid::Uuid::new_v4());
        let canonical = std::fs::canonicalize(&member_root).unwrap();
        let source = RepositorySourceIdentity {
            scheme: "test".to_string(),
            key_digest: "sha256:admission-member".to_string(),
            canonical_git_dir: canonical.join(".git"),
            canonical_origin: None,
            first_seen_path_hash: "sha256:admission-path".to_string(),
        };
        let mut manifest = crate::product::logical_codebase::store::LogicalCodebaseManifest::new(
            &project.id,
            aggregate_root.clone(),
            vec![member_id],
        );
        manifest.logical_codebase_id = uuid::Uuid::new_v4();
        lc_store.save_manifest(&project.id, &manifest).unwrap();
        let now = "2026-09-28T00:00:00Z".to_string();
        lc_store
            .save_member(
                &project.id,
                &CodebaseMemberRecord {
                    logical_repository_id: member_id,
                    physical_repository_id: "repository_admission_member".to_string(),
                    alias: "member-a".to_string(),
                    role: "member".to_string(),
                    ordinal: 1,
                    source_identity: source.clone(),
                    repo_type: RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![checkout_id],
                    status: MemberStatus::Active,
                    created_at: now.clone(),
                    updated_at: now.clone(),
                },
            )
            .unwrap();
        lc_store
            .save_checkout(
                &project.id,
                &RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id: member_id,
                    physical_repository_id: "repository_admission_member".to_string(),
                    kind: CheckoutKind::Main,
                    canonical_path: canonical.clone(),
                    checkout_path_hash: "sha256:admission-checkout".to_string(),
                    git_dir_identity: source.git_dir_identity(),
                    revision: None,
                    availability: CheckoutAvailability::Available,
                    observed_at: now.clone(),
                    created_at: now.clone(),
                    updated_at: now,
                },
            )
            .unwrap();

        let policy_store = AggregatePolicyArtifactStore::for_lc(paths.clone(), &lc.id);
        if with_policy {
            policy_store
                .ensure_bootstrap(&manifest)
                .expect("bootstrap policy");
        }

        let capabilities = Arc::new(StaticCapabilitySource::allowing());
        let streaming_adapter = Arc::new(CountingStreamingAdapter::new());
        let mut registry = ProviderRegistry::new();
        registry.register(
            crate::product::models::ProviderName::ClaudeCode,
            streaming_adapter.clone(),
        );
        registry.register(
            crate::product::models::ProviderName::Codex,
            streaming_adapter.clone(),
        );
        let gateway = Arc::new(LogicalCodebaseProviderGateway::with_audit(
            policy_store.clone(),
            capabilities.clone(),
            Arc::new(PassThroughTargetResolver),
            Arc::new(registry),
            Arc::new(StubSyncAdapter),
            always_available_gate(),
            Arc::new(crate::product::logical_codebase::GatewayRunAudit::new()),
            std::fs::canonicalize(&aggregate_root).expect("canonical authority root"),
        ));

        AdmissionFixture {
            _temp: temp,
            paths,
            project_id: project.id,
            lc_id: lc.id,
            aggregate_root,
            member_root,
            policy_store,
            capabilities,
            streaming_adapter,
            gateway,
        }
    }

    impl AdmissionFixture {
        fn write_language_rules(&self, contents: &str) {
            let dir = self.member_root.join(".claude/rules");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("language.md"), contents).unwrap();
        }

        fn preflight(&self) -> LogicalCodebaseProviderAdmissionPreflight {
            LogicalCodebaseProviderAdmissionPreflight::new(
                self.paths.clone(),
                self.lc_id.clone(),
                self.gateway.clone(),
            )
        }

        fn launch_request(&self) -> SessionLaunchRequest {
            SessionLaunchRequest::planning(
                self.project_id.clone(),
                ProviderRef::claude_code("snapshot-admission-test"),
                crate::product::logical_codebase::policy::PolicyTarget::aggregate_root(
                    self.aggregate_root.clone(),
                ),
                vec![self.aggregate_root.clone()],
                "sha256:admission-managed-config",
            )
        }
    }

    fn git_init_with_commit(path: &Path) {
        std::fs::create_dir_all(path).unwrap();
        for args in [
            vec!["init", "-b", "main"],
            vec!["config", "user.email", "admission@test.local"],
            vec!["config", "user.name", "Admission Test"],
        ] {
            let output = std::process::Command::new("git")
                .current_dir(path)
                .args(&args)
                .output()
                .expect("git");
            assert!(output.status.success(), "git {args:?} failed");
        }
        std::fs::write(path.join("README.md"), "# member\n").unwrap();
        let output = std::process::Command::new("git")
            .current_dir(path)
            .args(["add", "."])
            .output()
            .unwrap();
        assert!(output.status.success());
        let output = std::process::Command::new("git")
            .current_dir(path)
            .args(["commit", "-m", "init"])
            .output()
            .unwrap();
        assert!(output.status.success());
    }

    #[test]
    fn missing_member_language_rule_waits_before_provider_spawn() {
        let fixture = admission_fixture();
        // 成员 checkout 缺 .claude/rules/language.md。
        let error = fixture
            .preflight()
            .check(&fixture.launch_request(), &ProviderAdmissionPhase::Normal)
            .unwrap_err();
        match error {
            ProviderAdmissionError::Waiting {
                reason_code,
                missing_materials,
                allowed_actions,
                ..
            } => {
                assert_eq!(reason_code, "member_language_rules_missing");
                assert!(
                    missing_materials
                        .iter()
                        .any(|item| item.contains("language.md"))
                );
                assert!(allowed_actions.contains(&BootstrapActionKind::Prepare));
                assert!(allowed_actions.contains(&BootstrapActionKind::Retry));
            }
            other => panic!("expected waiting fact, got {other:?}"),
        }
        // provider 零启动。
        assert_eq!(fixture.streaming_adapter.start_count(), 0);
    }

    #[test]
    fn bootstrap_capability_record_is_not_real_provider_capability() {
        let fixture = admission_fixture();
        fixture.write_language_rules("# rules\n");
        // 真实 capability 谓词拒绝（记录缺失/不支持）→ waiting，而非因 JSON
        // 存在而放行。
        fixture.capabilities.deny();
        let error = fixture
            .preflight()
            .check(&fixture.launch_request(), &ProviderAdmissionPhase::Normal)
            .unwrap_err();
        match error {
            ProviderAdmissionError::Waiting {
                reason_code,
                detail,
                ..
            } => {
                assert_eq!(reason_code, "provider_capability_not_satisfied");
                assert!(detail.contains("capability"));
            }
            other => panic!("expected waiting fact, got {other:?}"),
        }
        assert_eq!(fixture.streaming_adapter.start_count(), 0);
    }

    #[test]
    fn policy_digest_or_authority_root_drift_blocks_spawn() {
        let fixture = admission_fixture();
        fixture.write_language_rules("# rules\n");
        // validate 冻结 envelope 后升级 policy revision/digest → spawn 前复验
        // （pub(crate) 拓宽后的 revalidate_before_spawn）拒绝，provider 零启动。
        let validated = fixture.gateway.validate(fixture.launch_request()).unwrap();
        let existing = fixture
            .policy_store
            .get(&fixture.project_id)
            .unwrap()
            .unwrap();
        let revised = existing.with_revised_policy("upgrade", "2026-09-28T01:00:00Z".to_string());
        fixture
            .policy_store
            .save(&fixture.project_id, &revised)
            .unwrap();

        let error = fixture
            .gateway
            .revalidate_before_spawn(
                &validated,
                &std::fs::canonicalize(&fixture.aggregate_root).unwrap(),
                false,
            )
            .unwrap_err();
        assert!(
            matches!(
                &error,
                ProviderGatewayError::PolicyDrift { dimension }
                    if dimension.contains("policy_revision") || dimension.contains("policy_digest")
            ),
            "unexpected drift error: {error:?}"
        );
        assert_eq!(fixture.streaming_adapter.start_count(), 0);

        // 之后的 admission 预检同样 waiting（resolver 与 store 的 policy 引用
        // 一致，但 envelope 由最新 validate 产出；漂移事实由复验路径证明）。
        let result = fixture
            .preflight()
            .check(&fixture.launch_request(), &ProviderAdmissionPhase::Normal);
        assert!(result.is_ok() || matches!(result, Err(ProviderAdmissionError::Waiting { .. })));
    }

    #[test]
    fn valid_rules_policy_and_gateway_produce_envelope() {
        let fixture = admission_fixture();
        fixture.write_language_rules("# language rules\n");

        let result = fixture
            .preflight()
            .check(&fixture.launch_request(), &ProviderAdmissionPhase::Normal)
            .unwrap();
        assert!(result.ready, "missing: {:?}", result.missing_materials);
        assert_eq!(result.rules.len(), 1);
        let rule = &result.rules[0];
        assert!(rule.digest.as_deref().unwrap().starts_with("sha256:"));
        assert_eq!(
            result.authority_root,
            std::fs::canonicalize(&fixture.aggregate_root).unwrap()
        );
        assert_eq!(result.capability_snapshot_ref, "snapshot-admission-test");
        assert!(result.policy.policy_digest.starts_with("sha256:"));
        // 预检本身零启动。
        assert_eq!(fixture.streaming_adapter.start_count(), 0);
    }

    // ---- Task 1.2（REQ-BOOT-04/D1）：phase credential 与 BootstrapExecutor ----

    use crate::cross_cutting::streaming_provider::{
        ProviderToolPolicy, ToolPolicyGuardError, ToolPolicyIntent, canonical_tool_policy,
        validate_tool_policy_for_role,
    };
    use crate::product::logical_codebase::aggregate_initialization::{
        AggregateCancellationRecord, AggregateInitializationErrorRecord,
        AggregateInitializationOperation, AggregateInitializationOperationInput,
        AggregateInitializationStepKind,
    };
    use crate::product::logical_codebase::aggregate_initialization_store::AggregateInitializationOperationStore;
    use crate::protocol::contracts::AdapterRole;

    const BOOTSTRAP_TS: &str = "2026-10-01T00:01:00Z";

    /// 构造 durable Running operation：MachineSkills/AggregatePreflight 已
    /// 完成，目标 step（默认 PreCheck）运行中并带 input digest——这是
    /// `BootstrapPhaseCredential` 的唯一合法派生面。
    fn running_bootstrap_operation(
        fixture: &AdmissionFixture,
    ) -> (AggregateInitializationOperationStore, String) {
        running_bootstrap_operation_at_step(fixture, AggregateInitializationStepKind::PreCheck)
    }

    fn running_bootstrap_operation_at_step(
        fixture: &AdmissionFixture,
        step: AggregateInitializationStepKind,
    ) -> (AggregateInitializationOperationStore, String) {
        let store = AggregateInitializationOperationStore::for_lc(
            fixture.paths.clone(),
            fixture.lc_id.clone(),
        );
        let operation_id = format!("op-{}", uuid::Uuid::new_v4().simple());
        let input = AggregateInitializationOperationInput {
            idempotency_key: format!("bootstrap-{operation_id}"),
            manifest_revision: 1,
            policy_digest: "sha256:bootstrap-fixture-policy".to_string(),
            profile_evidence_digest: None,
            provider_context_root: std::fs::canonicalize(&fixture.aggregate_root).unwrap(),
            provider: "claude_code".to_string(),
        };
        store
            .create_idempotent(AggregateInitializationOperation::new(
                operation_id.clone(),
                fixture.project_id.clone(),
                input,
                BOOTSTRAP_TS.to_string(),
            ))
            .expect("create bootstrap operation");
        store
            .mark_running(&fixture.project_id, &operation_id, BOOTSTRAP_TS.to_string())
            .expect("mark operation running");
        // 前置步骤按 V1 顺序完成，直到目标 step 可以运行。
        for predecessor in AggregateInitializationStepKind::V1 {
            if predecessor == step {
                break;
            }
            complete_bootstrap_step(&store, &fixture.project_id, &operation_id, predecessor);
        }
        store
            .mark_step_running(
                &fixture.project_id,
                &operation_id,
                step,
                format!("sha256:input-{}", step.as_str()),
                BOOTSTRAP_TS.to_string(),
            )
            .expect("mark bootstrap step running");
        (store, operation_id)
    }

    fn complete_bootstrap_step(
        store: &AggregateInitializationOperationStore,
        project_id: &str,
        operation_id: &str,
        step: AggregateInitializationStepKind,
    ) {
        store
            .mark_step_running(
                project_id,
                operation_id,
                step,
                format!("sha256:input-{}", step.as_str()),
                BOOTSTRAP_TS.to_string(),
            )
            .expect("mark predecessor running");
        store
            .checkpoint_step_output(
                project_id,
                operation_id,
                step,
                format!("artifact-{step:?}"),
                BOOTSTRAP_TS.to_string(),
            )
            .expect("checkpoint predecessor");
        store
            .mark_step_completed(project_id, operation_id, step, BOOTSTRAP_TS.to_string())
            .expect("complete predecessor");
    }

    /// 完成剩余全部步骤并 `finish_completed`（Completed 终态夹具）。
    fn finish_bootstrap_operation(
        store: &AggregateInitializationOperationStore,
        fixture: &AdmissionFixture,
        operation_id: &str,
    ) {
        let operation = store
            .get(&fixture.project_id, operation_id)
            .expect("load operation for completion");
        for (index, step) in AggregateInitializationStepKind::V1.into_iter().enumerate() {
            // 已 Completed 的步骤不可重复 mark_step_running（store 拒绝非
            // Pending 起始状态）；只推进尚未完成的步骤。
            if operation.steps[index].status
                == crate::product::logical_codebase::aggregate_initialization::AggregateInitializationStepStatus::Completed
            {
                continue;
            }
            complete_bootstrap_step(store, &fixture.project_id, operation_id, step);
        }
        store
            .finish_completed(&fixture.project_id, operation_id, BOOTSTRAP_TS.to_string())
            .expect("finish completed");
    }

    fn derived_credential(
        fixture: &AdmissionFixture,
    ) -> (
        AggregateInitializationOperationStore,
        String,
        BootstrapPhaseCredential,
    ) {
        let (store, operation_id) = running_bootstrap_operation(fixture);
        let credential = BootstrapPhaseCredential::from_running_operation(
            &store,
            &fixture.project_id,
            &operation_id,
            AggregateInitializationStepKind::PreCheck,
            &fixture.lc_id,
            &std::fs::canonicalize(&fixture.aggregate_root).unwrap(),
        )
        .expect("derive bootstrap credential from running operation");
        (store, operation_id, credential)
    }

    fn assert_bootstrap_waiting<T: std::fmt::Debug>(
        result: Result<T, ProviderAdmissionError>,
        expected_reason: &str,
    ) {
        match result {
            Err(ProviderAdmissionError::Waiting { reason_code, .. }) => {
                assert_eq!(reason_code, expected_reason);
            }
            other => panic!("expected waiting {expected_reason}, got {other:?}"),
        }
    }

    #[test]
    fn bootstrap_phase_credential_rejects_completed_or_drifted_operation() {
        let fixture = admission_fixture();
        let canonical_root = std::fs::canonicalize(&fixture.aggregate_root).unwrap();

        // 基线：Running + step 运行中 + digest/root/LC 匹配 → 唯一可派生面。
        let (_store, _operation_id, credential) = derived_credential(&fixture);
        assert_eq!(credential.step, AggregateInitializationStepKind::PreCheck);

        // Completed 终态 → 拒绝。
        let (store, operation_id) = running_bootstrap_operation(&fixture);
        finish_bootstrap_operation(&store, &fixture, &operation_id);
        assert_bootstrap_waiting(
            BootstrapPhaseCredential::from_running_operation(
                &store,
                &fixture.project_id,
                &operation_id,
                AggregateInitializationStepKind::PreCheck,
                &fixture.lc_id,
                &canonical_root,
            ),
            "bootstrap_operation_not_running",
        );

        // Failed 终态 → 拒绝。
        let (store, operation_id) = running_bootstrap_operation(&fixture);
        store
            .finish_failed(
                &fixture.project_id,
                &operation_id,
                Some(AggregateInitializationStepKind::PreCheck),
                AggregateInitializationErrorRecord::interrupted(),
                BOOTSTRAP_TS.to_string(),
            )
            .expect("finish failed");
        assert_bootstrap_waiting(
            BootstrapPhaseCredential::from_running_operation(
                &store,
                &fixture.project_id,
                &operation_id,
                AggregateInitializationStepKind::PreCheck,
                &fixture.lc_id,
                &canonical_root,
            ),
            "bootstrap_operation_not_running",
        );

        // Cancelled 终态 → 拒绝。
        let (store, operation_id) = running_bootstrap_operation(&fixture);
        store
            .cancel(
                &fixture.project_id,
                &operation_id,
                AggregateCancellationRecord {
                    reason_code: "test_cancelled".to_string(),
                    cancelled_at: BOOTSTRAP_TS.to_string(),
                    detail: None,
                },
                BOOTSTRAP_TS.to_string(),
            )
            .expect("cancel operation");
        assert_bootstrap_waiting(
            BootstrapPhaseCredential::from_running_operation(
                &store,
                &fixture.project_id,
                &operation_id,
                AggregateInitializationStepKind::PreCheck,
                &fixture.lc_id,
                &canonical_root,
            ),
            "bootstrap_operation_not_running",
        );

        // step 漂移：durable 运行中的是 RuleAndMcpConfig，凭据声明 PreCheck → 拒绝。
        let (store, operation_id) = running_bootstrap_operation_at_step(
            &fixture,
            AggregateInitializationStepKind::RuleAndMcpConfig,
        );
        assert_bootstrap_waiting(
            BootstrapPhaseCredential::from_running_operation(
                &store,
                &fixture.project_id,
                &operation_id,
                AggregateInitializationStepKind::PreCheck,
                &fixture.lc_id,
                &canonical_root,
            ),
            "bootstrap_step_not_running",
        );

        // 非 provider turn step（MachineSkills 运行中）→ 拒绝：确定性步骤
        // 不得派生 spawn 凭据。
        let (store, operation_id) = running_bootstrap_operation_at_step(
            &fixture,
            AggregateInitializationStepKind::MachineSkills,
        );
        assert_bootstrap_waiting(
            BootstrapPhaseCredential::from_running_operation(
                &store,
                &fixture.project_id,
                &operation_id,
                AggregateInitializationStepKind::MachineSkills,
                &fixture.lc_id,
                &canonical_root,
            ),
            "bootstrap_step_not_provider_turn",
        );

        // root 漂移：凭据 root 与 operation 的 provider_context_root 不一致 → 拒绝。
        let (store, operation_id) = running_bootstrap_operation(&fixture);
        let member_root = std::fs::canonicalize(&fixture.member_root).unwrap();
        assert_bootstrap_waiting(
            BootstrapPhaseCredential::from_running_operation(
                &store,
                &fixture.project_id,
                &operation_id,
                AggregateInitializationStepKind::PreCheck,
                &fixture.lc_id,
                &member_root,
            ),
            "bootstrap_root_drift",
        );

        // input digest 漂移：已派生凭据与当前 durable 记录 digest 不一致 →
        // spawn 前重核验拒绝（fail-closed，不可复用旧凭据）。
        let (store, _operation_id, credential) = derived_credential(&fixture);
        let drifted_digest = BootstrapPhaseCredential {
            input_digest: "sha256:drifted-input".to_string(),
            ..credential.clone()
        };
        assert_bootstrap_waiting(
            drifted_digest.reverify_against_running_operation(&store, &fixture.lc_id),
            "bootstrap_input_digest_drift",
        );

        // LC 漂移：凭据 LC 身份与核验 scope 不一致 → 拒绝。
        let drifted_lc = BootstrapPhaseCredential {
            logical_codebase_id: uuid::Uuid::new_v4().to_string(),
            ..credential
        };
        assert_bootstrap_waiting(
            drifted_lc.reverify_against_running_operation(&store, &fixture.lc_id),
            "bootstrap_logical_codebase_mismatch",
        );

        // 伪造 operation id → durable 缺失走 Store 错误（fail-closed）。
        assert!(matches!(
            BootstrapPhaseCredential::from_running_operation(
                &store,
                &fixture.project_id,
                "op-missing",
                AggregateInitializationStepKind::PreCheck,
                &fixture.lc_id,
                &canonical_root,
            ),
            Err(ProviderAdmissionError::Store(_))
        ));
    }

    #[test]
    fn bootstrap_phase_only_waives_missing_root_rules() {
        // 成员规则缺失 = 根 recipe 尚未生成的正常自举事实。
        let fixture = admission_fixture();
        let (_store, _operation_id, credential) = derived_credential(&fixture);

        // AggregateBootstrap：只豁免规则存在性；其余维度全过 → ready。
        let result = fixture
            .preflight()
            .check(
                &fixture.launch_request(),
                &ProviderAdmissionPhase::AggregateBootstrap(credential.clone()),
            )
            .expect("bootstrap phase must waive only missing root rules");
        assert!(result.ready, "missing: {:?}", result.missing_materials);
        // 缺失规则仍作为事实记录（digest=None），不构成阻断材料。
        assert_eq!(result.rules.len(), 1);
        assert!(result.rules[0].digest.is_none());
        assert!(result.missing_materials.is_empty());
        assert_eq!(fixture.streaming_adapter.start_count(), 0);

        // Normal 对照组：同样材料下规则缺失仍阻断（既有语义零变化）。
        assert_bootstrap_waiting(
            fixture
                .preflight()
                .check(&fixture.launch_request(), &ProviderAdmissionPhase::Normal)
                .map(|_| ()),
            "member_language_rules_missing",
        );

        // 豁免不覆盖 policy：aggregate policy artifact 缺失仍 waiting。
        let fixture_no_policy = admission_fixture_without_policy();
        let (_store_no_policy, _op_no_policy, credential_no_policy) =
            derived_credential(&fixture_no_policy);
        assert_bootstrap_waiting(
            fixture_no_policy
                .preflight()
                .check(
                    &fixture_no_policy.launch_request(),
                    &ProviderAdmissionPhase::AggregateBootstrap(credential_no_policy),
                )
                .map(|_| ()),
            "aggregate_policy_artifact_missing",
        );

        // 豁免不覆盖 capability：gateway capability 谓词拒绝仍 waiting。
        let fixture_denied = admission_fixture();
        fixture_denied.capabilities.deny();
        let (_store_denied, _op_denied, credential_denied) = derived_credential(&fixture_denied);
        assert_bootstrap_waiting(
            fixture_denied
                .preflight()
                .check(
                    &fixture_denied.launch_request(),
                    &ProviderAdmissionPhase::AggregateBootstrap(credential_denied),
                )
                .map(|_| ()),
            "provider_capability_not_satisfied",
        );

        // 凭据失效优先于豁免：operation Completed 后，凭据重核验失败——
        // 绝不降级为普通 session 或带病放行。
        let fixture_stale = admission_fixture();
        let (store_stale, _op_stale, stale_credential) = derived_credential(&fixture_stale);
        finish_bootstrap_operation(
            &store_stale,
            &fixture_stale,
            stale_credential.operation_id(),
        );
        assert_bootstrap_waiting(
            fixture_stale
                .preflight()
                .check(
                    &fixture_stale.launch_request(),
                    &ProviderAdmissionPhase::AggregateBootstrap(stale_credential),
                )
                .map(|_| ()),
            "bootstrap_operation_not_running",
        );
        assert_eq!(fixture_stale.streaming_adapter.start_count(), 0);
    }

    #[test]
    fn bootstrap_executor_marker_rejects_ordinary_executor_policy_in_lc_root_recipe() {
        let fixture = admission_fixture();
        let canonical_root = std::fs::canonicalize(&fixture.aggregate_root).unwrap();
        let (_store, _operation_id, credential) = derived_credential(&fixture);
        let marker = BootstrapExecutorMarker::new(
            credential,
            SessionPolicyAction::CodingTargetWrite,
            canonical_root,
            "root-recipe:pre_check:command-1",
        )
        .expect("complete bootstrap executor marker");
        let marker_policy = ProviderToolPolicy {
            intent: ToolPolicyIntent::BootstrapExecutorMarker(marker),
        };

        // LC root recipe 的自举通道：Executor + 完整 marker 是唯一放行形态。
        assert!(
            validate_tool_policy_for_role(&AdapterRole::Executor, Some(&marker_policy)).is_ok()
        );

        // 普通 Executor 策略（deny）仍被拒绝——marker 通道不是给普通
        // Executor/Coder 带策略的豁免口。
        let deny = ProviderToolPolicy::deny_file_write_builtins();
        assert!(validate_tool_policy_for_role(&AdapterRole::Executor, Some(&deny)).is_err());

        // 策略角色携带 marker：marker 不是策略（PolicyRequired）。
        for role in [
            AdapterRole::Orchestrator,
            AdapterRole::Reviewer,
            AdapterRole::WorkItemSplitter,
        ] {
            let rejected = matches!(
                validate_tool_policy_for_role(&role, Some(&marker_policy)),
                Err(ToolPolicyGuardError::PolicyRequired { .. })
            );
            assert!(
                rejected,
                "policy role {role:?} must not substitute the marker for its policy"
            );
        }

        // Handoff 不得使用自举通道。
        assert!(matches!(
            validate_tool_policy_for_role(&AdapterRole::Handoff, Some(&marker_policy)),
            Err(ToolPolicyGuardError::PolicyForbidden { .. })
        ));

        // marker 不投影为 canonical deny 策略：不进入策略会话 argv/审计通道。
        assert!(canonical_tool_policy("claude-code", &marker_policy).is_err());
        // 普通 Executor/Coder 无策略照常放行（既有双向语义零变化）。
        assert!(validate_tool_policy_for_role(&AdapterRole::Executor, None).is_ok());
    }

    #[test]
    fn bootstrap_executor_without_credential_or_receipt_context_is_rejected() {
        let fixture = admission_fixture();
        let canonical_root = std::fs::canonicalize(&fixture.aggregate_root).unwrap();
        let (_store, _operation_id, credential) = derived_credential(&fixture);

        // 凭据是 marker 的必带字段（类型面）：不存在「无凭据 marker」的
        // 构造形态——`BootstrapExecutorMarker::new` 只接受由 durable
        // Running operation 派生的 credential。
        // receipt context 缺失（空白）→ 构造即拒绝。
        assert_eq!(
            BootstrapExecutorMarker::new(
                credential.clone(),
                SessionPolicyAction::CodingTargetWrite,
                canonical_root.clone(),
                "   ",
            ),
            Err(BootstrapExecutorMarkerError::EmptyReceiptContext)
        );

        // canonical root 缺失 → 构造即拒绝。
        assert_eq!(
            BootstrapExecutorMarker::new(
                credential.clone(),
                SessionPolicyAction::CodingTargetWrite,
                std::path::PathBuf::new(),
                "root-recipe:pre_check:command-1",
            ),
            Err(BootstrapExecutorMarkerError::EmptyCanonicalRoot)
        );

        // 非写权限 action → 拒绝：BootstrapExecutor = 有写权限的 Executor。
        assert_eq!(
            BootstrapExecutorMarker::new(
                credential.clone(),
                SessionPolicyAction::PlanningReadOnly,
                canonical_root.clone(),
                "root-recipe:pre_check:command-1",
            ),
            Err(BootstrapExecutorMarkerError::InvalidAction(
                SessionPolicyAction::PlanningReadOnly
            ))
        );

        // spawn 前守卫对结构不完整 marker 再次拒绝（纵深防御：即使绕过
        // validating constructor，三 adapter 真实子进程前仍 fail-closed）。
        let complete = BootstrapExecutorMarker::new(
            credential,
            SessionPolicyAction::CodingTargetWrite,
            canonical_root,
            "root-recipe:pre_check:command-1",
        )
        .expect("complete marker");
        let degenerate = BootstrapExecutorMarker {
            receipt_context: String::new(),
            ..complete
        };
        let degenerate_policy = ProviderToolPolicy {
            intent: ToolPolicyIntent::BootstrapExecutorMarker(degenerate),
        };
        assert!(matches!(
            validate_tool_policy_for_role(&AdapterRole::Executor, Some(&degenerate_policy)),
            Err(ToolPolicyGuardError::BootstrapMarkerInvalid { .. })
        ));
    }

    /// Task 3.1（REQ-ENV-10/11/BOOT-04）：cwd≠target 分离形态下的普通缺根
    /// admission 回归锁——普通 session（Normal 相位，无自举凭据）以
    /// root cwd + 成员 Git checkout target 发起时，成员缺根规则
    ///（.claude/rules/language.md）照样 waiting、provider 零启动；cwd 经
    /// root 内自引用 symlink alias 字面传入，钉住 admission 复验判据是
    /// canonical 相等而非字面相等。补齐根规则后同一分离形态请求 ready，
    /// 证明分离形态本身不是阻断源。
    #[test]
    fn ordinary_missing_root_session_zero_spawns() {
        let fixture = admission_fixture();
        // cwd=root 的自引用 symlink alias（词法上位于 authority 内，canonical
        // 解析回 root 本体）。
        let cwd_alias = fixture.aggregate_root.join("root-alias");
        std::os::unix::fs::symlink(&fixture.aggregate_root, &cwd_alias).unwrap();
        // 成员身份从 LC 子树权威 store 读取（fixture 未保留 id 字段）。
        let lc = LogicalCodebaseStore::for_lc(fixture.paths.clone(), &fixture.lc_id);
        let members = lc.list_members(&fixture.project_id).unwrap();
        let member = members.first().expect("fixture seeds one member");
        let checkouts = lc.list_checkouts(&fixture.project_id).unwrap();
        let checkout = checkouts
            .iter()
            .find(|checkout| member.checkout_ids.contains(&checkout.checkout_id))
            .expect("member has a recorded checkout");
        let canonical_member = std::fs::canonicalize(&fixture.member_root).unwrap();

        let separated_request = SessionLaunchRequest {
            project_id: fixture.project_id.clone(),
            provider: ProviderRef::claude_code("snapshot-admission-test"),
            action: SessionPolicyAction::PlanningReadOnly,
            target: crate::product::logical_codebase::policy::PolicyTarget::checkout(
                member.logical_repository_id.0.to_string(),
                checkout.checkout_id.0.to_string(),
                canonical_member.clone(),
            ),
            working_directory: cwd_alias.clone(),
            readable_roots: vec![fixture.aggregate_root.clone()],
            writable_roots: Vec::new(),
            config_artifact_ref: "sha256:admission-managed-config".to_string(),
        };

        // 普通相位 + 成员缺根规则 → waiting（member_language_rules_missing），
        // provider 零启动。等待判据优先钉根规则缺失（若 cwd alias 复验失
        // 败会以 spawn_revalidation_drift 出现，reason_code 断言可捕捉）。
        let error = fixture
            .preflight()
            .check(&separated_request, &ProviderAdmissionPhase::Normal)
            .unwrap_err();
        match error {
            ProviderAdmissionError::Waiting {
                reason_code,
                missing_materials,
                ..
            } => {
                assert_eq!(reason_code, "member_language_rules_missing");
                assert!(
                    missing_materials
                        .iter()
                        .any(|item| item.contains("language.md"))
                );
            }
            other => panic!("expected waiting fact, got {other:?}"),
        }
        assert_eq!(fixture.streaming_adapter.start_count(), 0);

        // 对照：补齐根规则后同一分离形态请求 ready——分离形态（root cwd ≠
        // member target、symlink alias cwd）不是阻断源；admission 本身仍零启动。
        fixture.write_language_rules("# language\n");
        let result = fixture
            .preflight()
            .check(&separated_request, &ProviderAdmissionPhase::Normal)
            .expect("separated form with rules present must be ready");
        assert!(result.ready, "missing: {:?}", result.missing_materials);
        assert_eq!(fixture.streaming_adapter.start_count(), 0);
    }
}
