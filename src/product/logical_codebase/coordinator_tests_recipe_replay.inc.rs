    // 聚合初始化预检 + 五步 recipe 的生产同构装配：lc-scoped operation
    // store/preflight、gateway 审计链与注入桩。本文件经
    // coordinator_tests_profile_trust.inc.rs 在 mod tests 内 include，
    // 复用同模块的 gateway/审计链/注入桩。

    struct RecipeReplayFixture {
        _temp: tempfile::TempDir,
        paths: ProductAppPaths,
        lc_id: String,
        aggregate_root: PathBuf,
        manifest: LogicalCodebaseManifest,
        member_id: String,
        checkout_record: RepositoryCheckoutRecord,
        policy_digest: String,
        coordinator: AggregateInitializationCoordinator,
        service: DeterministicAggregatePreflightService,
        streaming_adapter: Arc<CountingStreamingAdapter>,
    }

    impl RecipeReplayFixture {
        /// 新 key 的 recipe input：与旧 operation 不同的 idempotency key。
        fn new_recipe_input(&self) -> AggregateInitializationOperationInput {
            AggregateInitializationOperationInput {
                idempotency_key: "recipe-replay-0002".to_string(),
                manifest_revision: self.manifest.membership_revision,
                policy_digest: self.policy_digest.clone(),
                profile_evidence_digest: Some("sha256:profile".to_string()),
                provider_context_root: self.aggregate_root.clone(),
                provider: "claude_code".to_string(),
            }
        }

        fn lc_store(&self) -> LogicalCodebaseStore {
            LogicalCodebaseStore::for_lc(self.paths.clone(), &self.lc_id)
        }

        /// 把成员 main checkout 指到新路径（边界重核验用）。
        fn point_member_checkout_at(&self, path: &Path) {
            let mut checkout = self.checkout_record.clone();
            checkout.canonical_path = path.to_path_buf();
            self.lc_store()
                .save_checkout("project_0001", &checkout)
                .unwrap();
        }

        fn inspect_root(&self) -> Result<AggregatePreflightSnapshot, AggregateInitializationError> {
            self.service
                .inspect("project_0001", &self.manifest, &CancellationToken::new())
        }
    }

    fn replay_git(cwd: &Path, arguments: &[&str]) {
        let output = std::process::Command::new("git")
            .current_dir(cwd)
            .args(arguments)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn replay_init_member(path: &Path) {
        std::fs::create_dir_all(path).unwrap();
        replay_git(path, &["init", "--quiet", "-b", "main"]);
        replay_git(path, &["config", "user.email", "replay@test.local"]);
        replay_git(path, &["config", "user.name", "Replay Test"]);
        std::fs::write(path.join("README.md"), "# member\n").unwrap();
        replay_git(path, &["add", "."]);
        replay_git(path, &["commit", "--quiet", "-m", "initial"]);
    }

    fn recipe_replay_fixture() -> RecipeReplayFixture {
        use crate::product::project_store::{CreateProjectInput, ProjectStore};

        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "recipe-replay".to_string(),
                description: None,
            })
            .unwrap();

        // 聚合根（非 Git）+ 一个真实成员 git 仓（main checkout）。
        let aggregate_root = temp.path().join("aggregate-root");
        std::fs::create_dir_all(&aggregate_root).unwrap();
        let member_root = aggregate_root.join("repo");
        replay_init_member(&member_root);

        let lc_id = LogicalCodebaseStore::new(paths.clone())
            .create(
                "project_0001",
                crate::product::logical_codebase::LogicalCodebaseCreateInput {
                    name: "recipe-replay-lc".to_string(),
                    aggregate_root: std::fs::canonicalize(&aggregate_root).unwrap(),
                },
            )
            .unwrap()
            .id;
        let lc_store = LogicalCodebaseStore::for_lc(paths.clone(), &lc_id);

        let member_id = LogicalRepositoryId(Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        let manifest =
            LogicalCodebaseManifest::new("project_0001", aggregate_root.clone(), vec![member_id]);
        lc_store.save_manifest("project_0001", &manifest).unwrap();
        lc_store
            .save_member(
                "project_0001",
                &CodebaseMemberRecord {
                    logical_repository_id: member_id,
                    physical_repository_id: "repository_repo".to_string(),
                    alias: "repo".to_string(),
                    role: "member".to_string(),
                    ordinal: 1,
                    source_identity: RepositorySourceIdentity::from_git_parts(
                        &member_root,
                        member_root.join(".git"),
                        Some("ssh://git@example.test/acme/repo.git".to_string()),
                    ),
                    repo_type: Default::default(),
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![checkout_id],
                    status: Default::default(),
                    created_at: "2026-10-02T00:00:00Z".to_string(),
                    updated_at: "2026-10-02T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        let checkout_record = RepositoryCheckoutRecord {
            checkout_id,
            logical_repository_id: member_id,
            physical_repository_id: "repository_repo".to_string(),
            kind: crate::product::logical_codebase::CheckoutKind::Main,
            canonical_path: member_root.clone(),
            checkout_path_hash: "sha256:checkout".to_string(),
            git_dir_identity: "sha256:git-dir".to_string(),
            revision: Some("abc123".to_string()),
            availability: crate::product::logical_codebase::CheckoutAvailability::Available,
            observed_at: "2026-10-02T00:00:00Z".to_string(),
            created_at: "2026-10-02T00:00:00Z".to_string(),
            updated_at: "2026-10-02T00:00:00Z".to_string(),
        };
        lc_store
            .save_checkout("project_0001", &checkout_record)
            .unwrap();

        // 存量自举桩 policy：正例语义的"旧 Completed 桩 operation"配套事实。
        let policy = crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::for_lc(
            paths.clone(),
            &lc_id,
        )
        .ensure_bootstrap(&manifest)
        .unwrap();
        assert!(
            policy.is_bootstrap_placeholder(),
            "fixture policy must stay the bootstrap placeholder"
        );

        // 本 scope 旧 operation：五步全走完，四命令审计与最终 receipt 照常
        // 在场（模拟上一次 recipe 执行留下的 durable 事实）。
        let old_operation_id = "aggregate_initialization_replay_old_0001".to_string();
        let init_store = AggregateInitializationOperationStore::for_lc(paths.clone(), &lc_id);
        init_store
            .create_idempotent(AggregateInitializationOperation::new(
                old_operation_id.clone(),
                "project_0001".to_string(),
                AggregateInitializationOperationInput {
                    idempotency_key: "replay-old-0001".to_string(),
                    manifest_revision: manifest.membership_revision,
                    policy_digest: policy.digest.clone(),
                    profile_evidence_digest: Some("sha256:profile".to_string()),
                    provider_context_root: aggregate_root.clone(),
                    provider: "claude_code".to_string(),
                },
                "2026-10-02T00:01:00Z".to_string(),
            ))
            .unwrap();
        init_store
            .mark_running(
                "project_0001",
                &old_operation_id,
                "2026-10-02T00:02:00Z".to_string(),
            )
            .unwrap();
        for step in AggregateInitializationStepKind::V1 {
            init_store
                .mark_step_running(
                    "project_0001",
                    &old_operation_id,
                    step,
                    format!("replay:{}:{}", old_operation_id, step.as_str()),
                    "2026-10-02T00:03:00Z".to_string(),
                )
                .unwrap();
            init_store
                .checkpoint_step_output(
                    "project_0001",
                    &old_operation_id,
                    step,
                    format!("aggregate-initializations/replay/{}.json", step.as_str()),
                    "2026-10-02T00:04:00Z".to_string(),
                )
                .unwrap();
            init_store
                .mark_step_completed(
                    "project_0001",
                    &old_operation_id,
                    step,
                    "2026-10-02T00:05:00Z".to_string(),
                )
                .unwrap();
        }
        init_store
            .finish_completed(
                "project_0001",
                &old_operation_id,
                "2026-10-02T00:06:00Z".to_string(),
            )
            .unwrap();

        // 旧 recipe 执行时根上已有 product-owned 入口：AGENTS.md + CLAUDE.md。
        std::fs::write(aggregate_root.join("AGENTS.md"), "# aggregate root rules\n").unwrap();
        std::fs::write(
            aggregate_root.join("CLAUDE.md"),
            "# claude root instructions\n",
        )
        .unwrap();
        let canonical_root = std::fs::canonicalize(&aggregate_root).unwrap();

        let receipt_store = crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(
            paths.clone(),
            lc_id.clone(),
        );

        // 真实 auditor 审计链：四命令前后全量快照、零未知变更 → 全部 Allowed。
        let auditor = crate::product::logical_codebase::RootRecipeFilesystemAuditor::new();
        for (index, (step, command_index, command)) in
            crate::product::logical_codebase::root_recipe_command_index()
                .into_iter()
                .enumerate()
        {
            let watch = auditor
                .before_command(
                    &old_operation_id,
                    &canonical_root,
                    step,
                    command_index,
                    command,
                )
                .unwrap();
            let receipt = auditor
                .after_command(watch, format!("2026-10-02T00:10:{index:02}Z"))
                .unwrap();
            assert_eq!(
                receipt.verdict,
                crate::product::logical_codebase::RootRecipeCommandVerdict::Allowed
            );
            receipt_store
                .append_command("project_0001", receipt)
                .unwrap();
        }
        let rule_digest =
            crate::product::logical_codebase::root_recipe_receipt::root_rule_digest(&canonical_root)
                .unwrap()
                .expect("evidence root must expose a rule digest");
        receipt_store
            .finalize(
                "project_0001",
                &old_operation_id,
                &policy.digest,
                &rule_digest,
                "2026-10-02T00:20:00Z".to_string(),
            )
            .unwrap();

        // 生产同构装配：lc-scoped operation store/preflight + gateway 录制链。
        let audit = Arc::new(GatewayRunAudit::new());
        let streaming_adapter = Arc::new(CountingStreamingAdapter::new(StreamingBehavior::Complete));
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderName::ClaudeCode, streaming_adapter.clone());
        let gateway = Arc::new(LogicalCodebaseProviderGateway::with_audit(
            crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::for_lc(
                paths.clone(),
                lc_id.clone(),
            ),
            Arc::new(StaticCapabilitySource::new("1.4.0")),
            Arc::new(PassthroughTargetResolver),
            Arc::new(registry),
            Arc::new(StubSyncAdapter),
            always_available_gate(),
            audit.clone(),
            aggregate_root.clone(),
        ));
        let provider: Arc<dyn AggregateProviderTurnDriver> = Arc::new(
            GatewayBackedAggregateProviderTurnDriver::claude_code(gateway, "cap_claude_code_1_4_0"),
        );
        let skills: Arc<dyn AggregateSkillsPreparation> = Arc::new(FakeSkillsPreparation {
            calls: Arc::new(Mutex::new(Vec::new())),
        });
        let service = DeterministicAggregatePreflightService::new(paths.clone()).for_lc(lc_id.clone());
        let preflight: Arc<dyn AggregatePreflightService> =
            Arc::new(DeterministicAggregatePreflightService::new(paths.clone()).for_lc(lc_id.clone()));
        let clock: Arc<Clock> = Arc::new(|| CREATED_AT.to_string());
        let coordinator = AggregateInitializationCoordinator::new(
            paths.clone(),
            AggregateInitializationOperationStore::new(paths.clone()),
            skills,
            preflight,
            provider,
            clock,
        )
        .for_lc(lc_id.clone());

        RecipeReplayFixture {
            _temp: temp,
            paths,
            lc_id,
            aggregate_root,
            manifest,
            member_id: member_id.0.to_string(),
            checkout_record,
            policy_digest: policy.digest,
            coordinator,
            service,
            streaming_adapter,
        }
    }

    #[tokio::test]
    async fn recipe_replay_preflight_completed_placeholder_root_advances_to_precheck() {
        let fixture = recipe_replay_fixture();

        // 独立确定性预检：根上已有 AGENTS.md/CLAUDE.md 也直接通过，不要求
        // 移动任何文件。
        let snapshot = fixture
            .inspect_root()
            .expect("scoped deterministic inspect must accept the initialized root");
        assert_eq!(snapshot.members.len(), 1);
        assert_eq!(snapshot.members[0].logical_repository_id, fixture.member_id);
        assert_eq!(
            PathBuf::from(&snapshot.aggregate_root),
            std::fs::canonicalize(&fixture.aggregate_root).unwrap()
        );

        // 新 key 通过 coordinator 建立新 operation 并完整推进：预检放行后
        // 进入 PreCheck provider turn，而不是停在 202/预检等待面。
        let operation = fixture
            .coordinator
            .begin(
                "aggregate_initialization_replay_new_0001".to_string(),
                "project_0001",
                fixture.new_recipe_input(),
            )
            .unwrap();
        let operation = fixture
            .coordinator
            .execute(
                "project_0001",
                &operation.operation_id,
                CancellationToken::new(),
            )
            .await
            .expect("initialized root must pass preflight and advance to PreCheck");
        assert_eq!(
            operation.status,
            AggregateInitializationOperationStatus::Completed
        );
        let preflight_step = operation
            .steps
            .get(AggregateInitializationStepKind::AggregatePreflight.index())
            .expect("aggregate_preflight step present");
        assert_eq!(
            preflight_step.status,
            AggregateInitializationStepStatus::Completed
        );

        // 录制的 provider 确实收到原 PreCheck 命令（命令文本唯一来源）;
        // 命令保持 prompt 逐字前缀（OracleAggInitScope scope 纪律线只追加）。
        use crate::product::repository_store::RepositoryInitializationStepKind as RepoStep;
        let inputs = fixture.streaming_adapter.started_inputs();
        assert_eq!(inputs.len(), 3, "three provider turns must run");
        assert!(inputs[0]
            .prompt
            .starts_with(RepoStep::PreCheck.command().unwrap()));
    }

    #[test]
    #[cfg(unix)]
    fn recipe_replay_preflight_revalidates_members_and_root_boundaries() {
        // 逐一破坏成员/root 边界：登记入口继续运行原检查，返回原对应错误，
        // 而不是沿用旧成员投影放行。错误面断言用统一辅助。
        fn expect_preflight_reason(fixture: &RecipeReplayFixture, expected: &str) {
            let error = fixture
                .inspect_root()
                .expect_err("boundary break must fail the replay preflight");
            let rendered = format!("{error:?}");
            assert!(
                rendered.contains(expected),
                "expected reason {expected}, got: {rendered}"
            );
        }

        // 1) 成员越界：checkout 指向 root 外的真实 git 仓。
        let fixture = recipe_replay_fixture();
        let outside = fixture._temp.path().join("outside-member");
        replay_init_member(&outside);
        fixture.point_member_checkout_at(&outside);
        expect_preflight_reason(&fixture, "member_path_outside_root");

        // 2) symlink escape：root 内 symlink 成员指向 root 外。
        let fixture = recipe_replay_fixture();
        let outside = fixture._temp.path().join("symlink-target");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, fixture.aggregate_root.join("escaping-member")).unwrap();
        fixture.point_member_checkout_at(&fixture.aggregate_root.join("escaping-member"));
        expect_preflight_reason(&fixture, "member_symlink_escape");

        // 3) linked worktree：成员仓挂出的 worktree 不能冒充 main checkout。
        let fixture = recipe_replay_fixture();
        let linked = fixture.aggregate_root.join("linked-wt");
        replay_git(
            &fixture.aggregate_root.join("repo"),
            &["worktree", "add", "--detach", linked.to_str().unwrap()],
        );
        fixture.point_member_checkout_at(&linked);
        expect_preflight_reason(&fixture, "nested_worktree");

        // 4) root 变 Git：预检提前拒绝，与旧成员投影无关。
        let fixture = recipe_replay_fixture();
        std::fs::create_dir_all(fixture.aggregate_root.join(".git")).unwrap();
        expect_preflight_reason(&fixture, "is a Git repository");

        // 5) root 重叠：另一 project 的 manifest 覆盖同一根。
        let fixture = recipe_replay_fixture();
        LogicalCodebaseStore::new(fixture.paths.clone())
            .save_manifest(
                "project_0002",
                &LogicalCodebaseManifest::new(
                    "project_0002",
                    std::fs::canonicalize(&fixture.aggregate_root).unwrap(),
                    Vec::new(),
                ),
            )
            .unwrap();
        expect_preflight_reason(&fixture, "aggregate_root_overlap");
    }
