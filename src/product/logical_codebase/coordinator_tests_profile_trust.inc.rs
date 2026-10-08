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
                _request: AggregateProviderTurnRequest<'_>,
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
        // turn）；OpenspecAndExamples=命令4。命令文本保持 prompt 逐字前缀
        // （OracleAggInitScope scope 纪律线只追加,不改命令冻结面）。
        assert!(inputs[0].prompt.starts_with(pre_check));
        assert!(inputs[1]
            .prompt
            .starts_with(&format!("{rule_config}\n{mcp_config}")));
        assert!(inputs[2].prompt.starts_with(rules_examples));

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

    /// OracleAggInitScope 红绿测(镜像 r54 worktree_discipline.rs 形态):
    /// r57 attempt1 cmd4 receipt 10 变更 unknown_path DENY 全落 alpha/beta
    /// (wrong-scope flavor),r57b attempt1 四命令零变更白烧(zero-action
    /// flavor)——streaming_input prompt 原为裸命令串拼接,聚合根 cwd 是
    /// 唯一隐式 scope 锚(oracle 定性 2026-10-08:与 r54 根因同构且更弱)。
    /// 本测试钉住:三个 root recipe turn 实际下发 prompt 都必须携带 scope
    /// 纪律段(真实 aggregate_root 绝对路径+成员仓负边界+auditor 逐路径
    /// 把关/fail-closed 后果+zero-action 正向句),且 recipe 命令文本保持
    /// 逐字前缀(命令同一性冻结面零变化)。
    #[tokio::test]
    async fn aggregate_recipe_prompt_carries_scope_discipline() {
        let fixture = gateway_aggregate_fixture();
        fixture
            .coordinator()
            .execute(
                "project_0001",
                "aggregate_initialization_0001",
                CancellationToken::new(),
            )
            .await
            .expect("aggregate recipe must complete");

        let inputs = fixture.streaming_inputs();
        assert_eq!(inputs.len(), 3, "three provider turns must launch");
        let canonical_root = std::fs::canonicalize(fixture.aggregate_root()).unwrap();
        for input in &inputs {
            super::assert_aggregate_scope_discipline(&input.prompt, &canonical_root);
        }

        // 命令冻结面:纪律段只追加,recipe 命令文本保持逐字前缀。
        use crate::product::repository_store::RepositoryInitializationStepKind as RepoStep;
        assert!(inputs[0]
            .prompt
            .starts_with(RepoStep::PreCheck.command().unwrap()));
        assert!(inputs[1].prompt.starts_with(&format!(
            "{}\n{}",
            RepoStep::RuleConfig.command().unwrap(),
            RepoStep::McpConfiguration.command().unwrap()
        )));
        assert!(inputs[2]
            .prompt
            .starts_with(RepoStep::ProjectRulesExamples.command().unwrap()));
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

    // Task 3（aggregate-policy-root-publication）：receipt 证明的重跑预检
    // 测试按主题拆入子 include（large_file_guard 1200 行上限，纯新增）。
    include!("coordinator_tests_recipe_replay.inc.rs");
