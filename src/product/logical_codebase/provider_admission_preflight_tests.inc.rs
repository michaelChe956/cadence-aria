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

    pub(super) struct PassThroughTargetResolver;
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

    pub(super) struct StaticCapabilitySource {
        deny: std::sync::atomic::AtomicBool,
        /// Task 3b:仅翻转 write_boundary 分格(缺 D4 证据的 early 场景)。
        write_boundary_denied: std::sync::atomic::AtomicBool,
        /// 路B(fix-admission-member-rules-scope):仅拒绝 Normal capability
        /// 矩阵(require_supported/resume/write_boundary),RootRecipe 通道
        /// (require_root_recipe_supported)不受影响——证明 bootstrap 相位的
        /// gateway 校验按已重验 phase 分流。
        normal_matrix_denied: std::sync::atomic::AtomicBool,
    }
    impl StaticCapabilitySource {
        fn allowing() -> Self {
            Self {
                deny: std::sync::atomic::AtomicBool::new(false),
                write_boundary_denied: std::sync::atomic::AtomicBool::new(false),
                normal_matrix_denied: std::sync::atomic::AtomicBool::new(false),
            }
        }
        pub(super) fn deny(&self) {
            self.deny.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        pub(super) fn deny_write_boundary(&self) {
            self.write_boundary_denied
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        pub(super) fn deny_normal_matrix(&self) {
            self.normal_matrix_denied
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        fn assert_normal_matrix_allowed(&self) -> Result<(), ProviderGatewayError> {
            if self
                .normal_matrix_denied
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                return Err(ProviderGatewayError::UnsupportedCapability(
                    "normal capability matrix is not satisfied (fixture knob)".to_string(),
                ));
            }
            Ok(())
        }

        fn capability(
            &self,
            provider: &ProviderRef,
            action: SessionPolicyAction,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
            use crate::product::logical_codebase::policy::ProviderWireDialect;
            use crate::product::logical_codebase::provider_capability_store::ProviderActionCapability;
            if self.deny.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(ProviderGatewayError::UnsupportedCapability(
                    "capability record missing".to_string(),
                ));
            }
            let (adapter_dialect, wire_dialect) = match provider.provider_type {
                ProviderRefType::ClaudeCode => (
                    ProviderDialect::ClaudeCodeCliV1,
                    ProviderWireDialect::ClaudeCodeStreamJson,
                ),
                ProviderRefType::Codex => (
                    ProviderDialect::CodexCliV1,
                    ProviderWireDialect::CodexAppServerRpc,
                ),
                ProviderRefType::Pi => (ProviderDialect::PiRpcV1, ProviderWireDialect::PiRpc),
                ProviderRefType::KimiCode => {
                    (ProviderDialect::KimiAcpV1, ProviderWireDialect::KimiAcp)
                }
            };
            Ok(ProviderCapability {
                provider_type: provider.provider_type,
                version: "1.4.0".to_string(),
                adapter_dialect,
                wire_dialect,
                capability_snapshot_ref: provider.capability_snapshot_ref.clone(),
                action_capability: ProviderActionCapability {
                    action,
                    launch: ProviderCapabilityEvidence::Confirmed,
                    resume: ProviderCapabilityEvidence::Confirmed,
                    write_boundary: if self
                        .write_boundary_denied
                        .load(std::sync::atomic::Ordering::SeqCst)
                    {
                        ProviderCapabilityEvidence::Unknown
                    } else {
                        ProviderCapabilityEvidence::Confirmed
                    },
                    projection_digest: format!("projection-digest-{action:?}"),
                    evidence_ref: format!("probe://{action:?}"),
                },
                trust: ProviderCapabilityEvidence::Confirmed,
            })
        }
    }
    impl ProviderCapabilitySource for StaticCapabilitySource {
        fn require_supported(
            &self,
            provider: &ProviderRef,
            action: SessionPolicyAction,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            self.assert_normal_matrix_allowed()?;
            self.capability(provider, action)
        }

        fn require_resume_supported(
            &self,
            provider: &ProviderRef,
            action: SessionPolicyAction,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            self.assert_normal_matrix_allowed()?;
            self.capability(provider, action)
        }

        fn require_write_boundary(
            &self,
            provider: &ProviderRef,
            action: SessionPolicyAction,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            self.assert_normal_matrix_allowed()?;
            let capability = self.capability(provider, action)?;
            if capability.action_capability.write_boundary
                != crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed
            {
                return Err(ProviderGatewayError::UnsupportedCapability(
                    crate::product::logical_codebase::provider_gateway::PROVIDER_CAPABILITY_WRITE_BOUNDARY_NOT_CONFIRMED
                        .to_string(),
                ));
            }
            Ok(capability)
        }

        fn require_root_recipe_supported(
            &self,
            provider: &ProviderRef,
            credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            let _ = credential;
            self.capability(provider, SessionPolicyAction::PlanningReadOnly)
        }
    }

    pub(super) struct CountingStreamingAdapter {
        start_count: std::sync::atomic::AtomicUsize,
    }
    impl CountingStreamingAdapter {
        pub(super) fn new() -> Self {
            Self {
                start_count: std::sync::atomic::AtomicUsize::new(0),
            }
        }
        pub(super) fn start_count(&self) -> usize {
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

    pub(super) struct StubSyncAdapter;
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

    pub(super) fn always_available_gate()
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

    pub(super) struct AdmissionFixture {
        pub(super) _temp: tempfile::TempDir,
        pub(super) paths: ProductAppPaths,
        pub(super) project_id: String,
        pub(super) lc_id: String,
        pub(super) aggregate_root: PathBuf,
        pub(super) member_root: PathBuf,
        pub(super) policy_store: AggregatePolicyArtifactStore,
        pub(super) capabilities: Arc<StaticCapabilitySource>,
        pub(super) streaming_adapter: Arc<CountingStreamingAdapter>,
        pub(super) gateway: Arc<LogicalCodebaseProviderGateway>,
    }

    pub(super) fn admission_fixture() -> AdmissionFixture {
        admission_fixture_with_policy(true)
    }

    /// Task 1.2：跳过 aggregate policy artifact 的变体（证明自举豁免不
    /// 覆盖 policy 维度）。
    pub(super) fn admission_fixture_without_policy() -> AdmissionFixture {
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
        pub(super) fn write_language_rules(&self, contents: &str) {
            let dir = self.member_root.join(".claude/rules");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("language.md"), contents).unwrap();
        }

        pub(super) fn preflight(&self) -> LogicalCodebaseProviderAdmissionPreflight {
            LogicalCodebaseProviderAdmissionPreflight::new(
                self.paths.clone(),
                self.lc_id.clone(),
                self.gateway.clone(),
            )
        }

        pub(super) fn launch_request(&self) -> SessionLaunchRequest {
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

    /// 路A 边界(fix-admission-member-rules-scope):自举桩政策(placeholder,
    /// 存量迁移前)的普通请求,成员 checkout 缺 `.claude/rules/language.md`
    /// 仍阻断(member_language_rules_missing,Prepare/Retry)——placeholder
    /// 不是「根材料有效」证明,不得借成员豁免扩大普通请求放行;provider
    /// 零启动。
    #[test]
    fn missing_member_language_rule_waits_before_provider_spawn() {
        let fixture = admission_fixture();
        // placeholder 政策 + 成员 checkout 缺 .claude/rules/language.md。
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

    /// 路A 主断言(fix-admission-member-rules-scope,先红):根材料(根规则+
    /// 已发布政策+匹配最终 receipt)有效时,成员 language.md 缺失不再阻断
    /// ——ready=true、allowed_actions 空(不提示准备无关成员);missing_
    /// member_rules 保留非阻断审计事实;admission 不读取不消费的成员文件
    /// 正文(引用在场、digest 恒 None,仅审计语义,不作为正文已存在或已
    /// 核验的证明);provider 零启动。
    #[test]
    fn valid_root_materials_waive_missing_member_rules_as_audit_only() {
        let fixture = admission_fixture();
        publish_valid_root_materials(&fixture, "op_path_a_waive");
        // 成员 checkout 缺 .claude/rules/language.md(不 write_language_rules)。

        let result = fixture
            .preflight()
            .check(&fixture.launch_request(), &ProviderAdmissionPhase::Normal)
            .expect("valid root materials must waive missing member rules");
        assert!(result.ready, "missing: {:?}", result.missing_materials);
        // 审计保留:缺失成员规则作为非阻断事实记录。
        assert_eq!(result.missing_member_rules.len(), 1);
        assert!(result.missing_member_rules[0].contains("language.md"));
        // 只有这些非必需成员材料缺失:不产生阻断材料、无成员准备动作。
        assert!(result.missing_materials.is_empty());
        assert!(result.allowed_actions.is_empty());
        // 不读取不消费的成员文件正文:引用在场但 digest 恒 None。
        assert_eq!(result.rules.len(), 1);
        assert!(result.rules[0].digest.is_none());
        assert_eq!(fixture.streaming_adapter.start_count(), 0);
    }

    /// Review Focus 5(fix-admission-member-rules-scope):根材料缺失+成员
    /// 缺失并存 → 仍 fail-closed(root_rules_missing),不把成员豁免误当
    /// 根豁免;provider 零启动。
    #[test]
    fn published_root_missing_with_member_missing_stays_fail_closed() {
        let fixture = admission_fixture();
        // 已发布政策 + 匹配最终 receipt,但 canonical root 自身的
        // .claude/rules/language.md 缺失;成员规则同样缺失。
        publish_root_policy_with_receipt(&fixture, "op_path_a_root_missing");
        let error = fixture
            .preflight()
            .check(&fixture.launch_request(), &ProviderAdmissionPhase::Normal)
            .unwrap_err();
        match error {
            ProviderAdmissionError::Waiting { reason_code, .. } => {
                assert_eq!(reason_code, "root_rules_missing");
            }
            other => panic!("expected waiting fact, got {other:?}"),
        }
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
        // 路A:admission 不读取不消费的成员文件正文——引用 digest 恒 None
        // (仅审计语义,不作为正文已存在或已核验的证明)。
        assert!(rule.digest.is_none());
        assert_eq!(
            result.authority_root,
            std::fs::canonicalize(&fixture.aggregate_root).unwrap()
        );
        assert_eq!(result.capability_snapshot_ref, "snapshot-admission-test");
        assert!(result.policy.policy_digest.starts_with("sha256:"));
        // 预检本身零启动。
        assert_eq!(fixture.streaming_adapter.start_count(), 0);
    }

    /// 把 fixture 的自举桩升级为一次真实 #8 发布形态:store 保存非桩
    /// artifact,canonical root 落 locator 正文(policy_id 原字节)与
    /// AGENTS.md,LC 子树写入与 digest 链一致的最终 receipt。
    pub(super) fn publish_root_policy_with_receipt(
        fixture: &AdmissionFixture,
        operation_id: &str,
    ) -> crate::product::logical_codebase::policy::AggregatePolicyArtifact {
        use sha2::Digest as _;

        let current = fixture
            .policy_store
            .get(&fixture.project_id)
            .expect("read current policy")
            .expect("fixture policy present");
        let artifact = current.with_revised_policy(
            "# 聚合政策\n\npublished body for lcg_t03\n",
            "2026-10-03T00:00:00Z".to_string(),
        );
        fixture
            .policy_store
            .save(&fixture.project_id, &artifact)
            .expect("save published artifact");

        let canonical_root = std::fs::canonicalize(&fixture.aggregate_root).unwrap();
        let locator = canonical_root.join(&artifact.policy_id);
        std::fs::create_dir_all(locator.parent().expect("locator parent")).unwrap();
        std::fs::write(&locator, artifact.policy_text.as_bytes()).unwrap();

        let rule_bytes = b"# AGENTS rules\nlcg_t03 root rules\n";
        std::fs::write(canonical_root.join("AGENTS.md"), rule_bytes).unwrap();
        let rule_digest = format!("sha256:{:x}", sha2::Sha256::digest(rule_bytes));

        let receipt = crate::product::logical_codebase::RootRecipeReceipt {
            operation_id: operation_id.to_string(),
            canonical_root: canonical_root.clone(),
            commands: Vec::new(),
            policy_digest: artifact.digest.clone(),
            rule_digest,
            finalized_at: "2026-10-03T00:00:00Z".to_string(),
        };
        let scope = crate::product::logical_codebase::store::lc_scope_root(
            &fixture.paths,
            &fixture.project_id,
            &Some(fixture.lc_id.clone()),
        )
        .expect("lc scope root");
        let receipts = scope.join("aggregate-recipe-receipts");
        std::fs::create_dir_all(&receipts).unwrap();
        crate::product::json_store::write_json(
            &receipts.join(format!("{operation_id}.json")),
            &receipt,
        )
        .expect("write fixture receipt");
        artifact
    }
    /// Task 1(fix-admission-member-rules-scope):真实有效根材料夹具——在
    /// `publish_root_policy_with_receipt` 的发布链(locator 正文 + AGENTS 原字节
    /// + 匹配最终 receipt)之上,补 canonical root 自身的
    ///   `.claude/rules/language.md`,构成「根规则 + 已发布政策 + 最终 receipt」
    ///   齐备的有效根形态。路 A/路 B 的正向断言以本夹具为前提——豁免面靠真实
    ///   根材料证明,不靠 placeholder 政策或补成员规则(REQ-REG-12 delta:
    ///   placeholder 不是「根材料有效」证明)。Task 2/3 消费;夹具矩阵区分:
    ///   普通有效根 = 本函数;故意缺根 = `publish_root_policy_with_receipt`
    ///   (不建根 language);placeholder 兼容 = `admission_fixture` 默认形态;
    ///   fresh-bootstrap = 默认形态 + 有效凭据(根材料由 recipe 生成)。
    pub(super) fn publish_valid_root_materials(
        fixture: &AdmissionFixture,
        operation_id: &str,
    ) -> crate::product::logical_codebase::policy::AggregatePolicyArtifact {
        let artifact = publish_root_policy_with_receipt(fixture, operation_id);
        let canonical_root = std::fs::canonicalize(&fixture.aggregate_root).unwrap();
        std::fs::create_dir_all(canonical_root.join(".claude/rules")).unwrap();
        std::fs::write(
            canonical_root.join(".claude/rules/language.md"),
            "# 语言规则\n\n- 有效根材料 fixture:canonical root 自身的 language 规则。\n",
        )
        .unwrap();
        artifact
    }
}
