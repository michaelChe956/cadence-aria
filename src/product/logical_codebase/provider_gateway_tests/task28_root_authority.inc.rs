// Task 2.8（映射 tasks.md 2.2）：双工厂 root assertion 与 cwd authority 的
// 测试体（由 provider_gateway_tests.rs 的 task28_gateway_root_authority
// 模块 include，保持主文件低于 large_file_guard 1200 行上限）。
    use super::*;
    use crate::product::logical_codebase::store::LogicalCodebaseRecord;
    use crate::product::logical_codebase::{
        LogicalCodebaseStore, RootRecipeReceipt, assert_canonical_lc_root_consistent,
    };
    // Task 3.1（cwd≠target 专用回归锁 fixture）追加依赖。
    use crate::product::logical_codebase::policy::AggregatePolicyArtifactStore;
    use crate::product::logical_codebase::production_policy_resolvers::ProductionPolicyTargetResolver;
    use crate::product::logical_codebase::{
        CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, GatewayRunAudit,
        IdentityRegistryEntry, IdentityRegistryStore, LogicalCodebaseCreateInput,
        LogicalRepositoryId, MemberStatus, RepositoryCheckoutId, RepositoryCheckoutRecord,
        RepositorySourceIdentity, RepositoryType,
    };
    use crate::product::project_store::{CreateProjectInput, ProjectStore};
    use uuid::Uuid;
    use crate::web::gateway_factory::LogicalCodebaseGatewayFactory;

    /// 构造生产形态的 gateway 工厂（ClaudeCode registry + stub adapter +
    /// 始终可用 gate），供双工厂 root assertion 测试组装 gateway。
    fn root_authority_factory(paths: &ProductAppPaths) -> Arc<LogicalCodebaseGatewayFactory> {
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderName::ClaudeCode, Arc::new(CountingStreamingAdapter::new()));
        Arc::new(LogicalCodebaseGatewayFactory::new(
            paths.clone(),
            Arc::new(registry),
            Arc::new(StubSyncAdapter),
            always_available_gate(),
        ))
    }

    /// 写登记工厂的 record.json 投影（aggregate_root 即登记时冻结的根）。
    fn write_lc_record(
        paths: &ProductAppPaths,
        project_id: &str,
        lc_id: &str,
        aggregate_root: &Path,
    ) {
        let record_root = paths.logical_codebase_record_root(project_id, lc_id);
        std::fs::create_dir_all(&record_root).expect("create lc record root");
        let record = LogicalCodebaseRecord {
            id: lc_id.to_string(),
            name: "lc".to_string(),
            aggregate_root: aggregate_root.to_path_buf(),
            created_at: "2026-10-01T00:00:00Z".to_string(),
        };
        crate::product::json_store::write_json(&record_root.join("record.json"), &record)
            .expect("write lc record.json");
    }

    /// 写 aggregate 生产 driver 的 root recipe receipt 投影（canonical_root
    /// 即 preflight snapshot root 经命令审计冻结的根）。
    fn write_root_recipe_receipt(
        paths: &ProductAppPaths,
        project_id: &str,
        lc_id: &str,
        operation_id: &str,
        canonical_root: &Path,
    ) {
        let scope = crate::product::logical_codebase::store::lc_scope_root(
            paths,
            project_id,
            &Some(lc_id.to_string()),
        )
        .expect("lc scope root");
        let receipts = scope.join("aggregate-recipe-receipts");
        std::fs::create_dir_all(&receipts).expect("create receipts root");
        let receipt = RootRecipeReceipt {
            operation_id: operation_id.to_string(),
            canonical_root: canonical_root.to_path_buf(),
            commands: Vec::new(),
            policy_digest: "sha256:policy".to_string(),
            rule_digest: "sha256:rule".to_string(),
            finalized_at: "2026-10-01T00:00:00Z".to_string(),
        };
        crate::product::json_store::write_json(
            &receipts.join(format!("{operation_id}.json")),
            &receipt,
        )
        .expect("write root recipe receipt");
    }

    /// 显式 cwd 的 planning 只读请求（cwd 与 target 独立传递）。
    fn planning_request_with_cwd(
        project_id: &str,
        cwd: PathBuf,
        target_worktree: PathBuf,
        readable_root: PathBuf,
    ) -> SessionLaunchRequest {
        SessionLaunchRequest {
            project_id: project_id.to_string(),
            provider: ProviderRef::claude_code("cap_claude_code_1_4_0"),
            action: SessionPolicyAction::PlanningReadOnly,
            target: PolicyTarget::checkout(
                "logical_repo_0001",
                "checkout_0001",
                target_worktree,
            ),
            working_directory: cwd,
            readable_roots: vec![readable_root],
            writable_roots: Vec::new(),
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
        }
    }

    /// 双工厂 root assertion：四路 canonical root 投影一致才放行；任一工厂
/// 投影漂移（登记根 / aggregate driver receipt 根 / envelope cwd）都
    /// fail-closed。fixture 用带空格目录 + symlink 证明判据是 canonical
    /// 相等，不是字面相等；投影缺失（bootstrap 早期）不视为不一致。
    #[test]
    fn dual_gateway_factories_reject_canonical_root_mismatch() {
        let root = tempfile::tempdir().expect("temporary root");
        let paths = ProductAppPaths::new(root.path().to_path_buf());
        let project_id = "project_0001";
        let lc_id = "lc_root_assert";

        // 带空格的 canonical LC root + 指向它的 symlink：两路投影字面不同。
        let lc_root = root.path().join("lc root");
        std::fs::create_dir_all(&lc_root).expect("create lc root");
        let lc_root_link = root.path().join("lc-root-link");
        std::os::unix::fs::symlink(&lc_root, &lc_root_link).expect("symlink lc root");
        let other_root = root.path().join("other root");
        std::fs::create_dir_all(&other_root).expect("create other root");
        let canonical_lc_root = lc_root.canonicalize().unwrap();

        // Part A：纯断言函数——登记(plain)/aggregate(symlink)/authority(symlink)/
        // envelope cwd(plain) canonical 相等才放行，返回 canonical root。
        let consistent = assert_canonical_lc_root_consistent(
            Some(&lc_root),
            Some(&lc_root_link),
            &lc_root_link,
            &lc_root,
        )
        .expect("canonically equal projections must pass");
        assert_eq!(consistent, canonical_lc_root);

        // 投影缺失不视为不一致（bootstrap 早期/legacy 无 receipt）。
        assert_canonical_lc_root_consistent(None, None, &lc_root, &lc_root)
            .expect("absent projections are not mismatches");

        // 登记工厂投影漂移 → fail-closed。
        assert!(
            matches!(
                assert_canonical_lc_root_consistent(
                    Some(&other_root),
                    Some(&lc_root),
                    &lc_root,
                    &lc_root
                ),
                Err(ProviderGatewayError::TargetMismatch { ref field })
                    if field == "registration_root"
            ),
            "registration root drift must fail closed"
        );
        // aggregate 生产 driver 投影漂移 → fail-closed。
        assert!(
            matches!(
                assert_canonical_lc_root_consistent(
                    Some(&lc_root),
                    Some(&other_root),
                    &lc_root,
                    &lc_root
                ),
                Err(ProviderGatewayError::TargetMismatch { ref field })
                    if field == "aggregate_root"
            ),
            "aggregate driver root drift must fail closed"
        );
        // envelope cwd 与 authority root 不一致（root-cwd 契约破坏）→ fail-closed。
        assert!(
            matches!(
                assert_canonical_lc_root_consistent(
                    Some(&lc_root),
                    Some(&lc_root),
                    &lc_root,
                    &other_root
                ),
                Err(ProviderGatewayError::TargetMismatch { ref field })
                    if field == "lc_root"
            ),
            "envelope cwd drift from authority root must fail closed"
        );

        // Part B：工厂层——manifest authority root 与登记/receipt 投影不一致时
        // build_for_lc fail-closed（gateway 不产出 ⇒ 后续所有 spawn 为零）。
        let factory = root_authority_factory(&paths);
        let manifest = LogicalCodebaseManifest::new(project_id, lc_root_link.clone(), vec![]);
        LogicalCodebaseStore::for_lc(paths.clone(), lc_id)
            .save_manifest(project_id, &manifest)
            .expect("save manifest");

        // 登记投影与 manifest 根一致（字面经 symlink，canonical 相等）→ 组装成功。
        write_lc_record(&paths, project_id, lc_id, &lc_root);
        factory
            .build_for_lc(project_id, Some(lc_id))
            .expect("consistent projections must assemble a gateway");

        // 登记投影漂移 → 组装失败（zero spawn 边界前移到工厂）。
        write_lc_record(&paths, project_id, lc_id, &other_root);
        assert!(
            matches!(
                factory.build_for_lc(project_id, Some(lc_id)),
                Err(ProviderGatewayError::TargetMismatch { ref field })
                    if field == "registration_root"
            ),
            "registration root drift must reject gateway assembly"
        );

        // receipt（aggregate driver 投影）漂移 → 组装失败；登记投影修正回
        // 一致后仅剩 receipt 漂移作为唯一判据。
        write_lc_record(&paths, project_id, lc_id, &lc_root);
        write_root_recipe_receipt(&paths, project_id, lc_id, "op_0001", &other_root);
        assert!(
            matches!(
                factory.build_for_lc(project_id, Some(lc_id)),
                Err(ProviderGatewayError::TargetMismatch { ref field })
                    if field == "aggregate_root"
            ),
            "aggregate driver receipt root drift must reject gateway assembly"
        );

        // receipt 与 manifest 根 canonical 一致（经 symlink 形态）→ 组装恢复。
        write_root_recipe_receipt(&paths, project_id, lc_id, "op_0001", &lc_root_link);
        factory
            .build_for_lc(project_id, Some(lc_id))
            .expect("canonically consistent receipt must assemble a gateway");
    }

    /// cwd authority 与 Git identity：cwd 越出 authority 在 validate 即拒绝；
    /// symlink 逃逸在 spawn 前 canonical 复验拒绝；Git identity 漂移继续
    /// zero spawn；cwd≠target 的合法分离形态（root cwd + member target）
    /// validate 与 spawn 均放行。
    #[test]
    fn cwd_outside_authority_or_target_git_identity_drift_zero_spawns() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let project_id = fixture.manifest().project_id;
        let authority = fixture.paths.root().to_path_buf();
        let worktree = fixture.real_worktree();

        // (a) cwd 越出 authority（外来根）→ validate 即 fail-closed，零 spawn。
        let outside = tempfile::tempdir().expect("outside authority root");
        let request = planning_request_with_cwd(
            &project_id,
            outside.path().to_path_buf(),
            worktree.clone(),
            authority.clone(),
        );
        assert!(
            matches!(
                fixture.gateway().validate(request),
                Err(ProviderGatewayError::TargetMismatch { ref field })
                    if field == "cwd_authority"
            ),
            "cwd outside authority must fail closed at validate"
        );
        assert_eq!(fixture.registry_start_count(), 0);

        // (b) symlink 逃逸：cwd 路径词法上位于 authority 内（validate 早门
        // 放行），canonical 解析到界外 → spawn 前复验 fail-closed，零 spawn。
        let escape_link = authority.join("escape-link");
        std::os::unix::fs::symlink(outside.path(), &escape_link).expect("symlink escape");
        let request = planning_request_with_cwd(
            &project_id,
            escape_link.clone(),
            worktree.clone(),
            authority.clone(),
        );
        let validated = fixture
            .gateway()
            .validate(request)
            .expect("lexical gate passes; canonical escape is caught at spawn");
        let launch = ValidatedStreamingProviderInput::new(
            fixture.streaming_input(escape_link, None),
            validated,
        );
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let spawned = runtime
            .block_on(fixture.gateway().start_streaming(launch, CancellationToken::new()));
        let error = match spawned {
            Err(error) => error,
            Ok(_) => panic!("canonical escape must fail closed before spawn"),
        };
        assert!(
            matches!(
                &error,
                ProviderGatewayError::TargetMismatch { field } if field == "cwd_authority"
            ),
            "expected cwd_authority mismatch, got {error:?}"
        );
        assert_eq!(fixture.registry_start_count(), 0);

        // (c) Git identity 漂移（.git 指针被调包）→ validate 即拒绝，零 spawn。
        let request = planning_request_with_cwd(
            &project_id,
            authority.clone(),
            worktree.clone(),
            authority.clone(),
        );
        fixture
            .targets()
            .change_git_dir_after_request("/work/api/.git-replaced");
        assert!(
            matches!(
                fixture.gateway().validate(request),
                Err(ProviderGatewayError::TargetMismatch { ref field })
                    if field == "git_dir"
            ),
            "git identity drift must keep failing closed"
        );
        assert_eq!(fixture.registry_start_count(), 0);

        // (d) 合法分离形态：cwd=authority root ≠ member target → 放行并真实
        // 启动（旧的 cwd==target 等式会错杀该形态）。
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let project_id = fixture.manifest().project_id;
        let authority = fixture.paths.root().to_path_buf();
        let worktree = fixture.real_worktree();
        let request = planning_request_with_cwd(
            &project_id,
            authority.clone(),
            worktree.clone(),
            authority.clone(),
        );
        let validated = fixture
            .gateway()
            .validate(request)
            .expect("root cwd with member target is the legal split form");
        let launch = ValidatedStreamingProviderInput::new(
            fixture.streaming_input(authority, None),
            validated,
        );
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime
            .block_on(fixture.gateway().start_streaming(launch, CancellationToken::new()))
            .expect("legal cwd!=target form must spawn");
        assert_eq!(fixture.registry_start_count(), 1);
    }

    /// 写证据不随 cwd 扩大：coding 的唯一 writable root 恰为 target
    /// worktree；root cwd 不进入写证据（REQ-ENV-03），provider 消费的写
    /// 证据来自 envelope 冻结值——evidence gate 只作证据，不猜 OS sandbox。
    #[test]
    fn provider_specific_writable_evidence_does_not_expand_coding_root() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let project_id = fixture.manifest().project_id;
        let authority = fixture.paths.root().to_path_buf();
        let worktree = fixture.real_worktree();

        // coding 请求 cwd=root（≠target）：envelope 冻结的写证据仍恰为
        // [target worktree]，root cwd 没有扩大写根。
        let request = SessionLaunchRequest {
            project_id: project_id.clone(),
            provider: ProviderRef::claude_code("cap_claude_code_1_4_0"),
            action: SessionPolicyAction::CodingTargetWrite,
            target: PolicyTarget::checkout(
                "logical_repo_0001",
                "checkout_0001",
                worktree.clone(),
            ),
            working_directory: authority.clone(),
            readable_roots: vec![authority.clone()],
            writable_roots: vec![worktree.clone()],
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
        };
        let validated = fixture
            .gateway()
            .validate(request)
            .expect("root cwd coding request must validate");
        assert_eq!(validated.envelope().writable_roots, vec![worktree.clone()]);

        // 试图把 root cwd 当写证据 → envelope fail-closed（证据门不放宽）。
        let mut expanded = validated_request_snapshot(&fixture, &project_id, &authority, &worktree);
        expanded.writable_roots = vec![authority.clone()];
        assert!(
            matches!(
                fixture.gateway().validate(expanded),
                Err(ProviderGatewayError::Policy(_))
            ),
            "writable evidence beyond the target worktree must fail closed"
        );

        // sync spawn：AdapterInput 的独立 working_directory=root 优先于
        // worktree_path，spawn 复验按 root cwd 通过；写证据仍只含 target。
        let adapter_input = AdapterInput {
            working_directory: Some(authority.clone()),
            provider_type: crate::protocol::contracts::ProviderType::ClaudeCode,
            role: crate::protocol::contracts::AdapterRole::WorkItemSplitter,
            worktree_path: Some(worktree.to_string_lossy().into_owned()),
            provider_stream_log_dir: None,
            prompt: "coding probe".to_string(),
            context_files: Vec::new(),
            output_schema: String::new(),
            timeout: 1,
            max_retries: 0,
        };
        let launch = ValidatedAdapterInput::new(adapter_input, validated);
        fixture
            .gateway()
            .run_sync(launch)
            .expect("independent cwd must pass spawn revalidation");
        assert_eq!(fixture.gateway_audit().sync_launches(), 1);
    }

    /// 重建一份 coding 请求（供写证据越权用例修改 writable_roots）。
    fn validated_request_snapshot(
        fixture: &GatewayFixture,
        project_id: &str,
        authority: &Path,
        worktree: &Path,
    ) -> SessionLaunchRequest {
        SessionLaunchRequest {
            project_id: project_id.to_string(),
            provider: ProviderRef::claude_code("cap_claude_code_1_4_0"),
            action: SessionPolicyAction::CodingTargetWrite,
            target: PolicyTarget::checkout(
                "logical_repo_0001",
                "checkout_0001",
                worktree.to_path_buf(),
            ),
            working_directory: authority.to_path_buf(),
            readable_roots: vec![fixture.paths.root().to_path_buf()],
            writable_roots: vec![worktree.to_path_buf()],
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
        }
    }

    // ---- Task 3.1（REQ-ENV-10/11/BOOT-04 映射）：cwd≠target 专用回归锁
    // fixture 与 resume 指纹维度锁。生产逻辑已由 Phase 2 全量落地，此处
    // 只以真实形态钉住契约，不引入新生产代码。----

    fn run_git(cwd: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .current_dir(cwd)
            .args(args)
            .output()
            .expect("git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// 分离形态专用 deterministic fixture：非 Git canonical LC root（路径含
    /// 空格，登记/manifest 投影经 symlink alias 字面）+ 成员主仓真实 Git
    /// checkout（.git 目录）+ 真实 `git worktree add` 链接工作树（.git 文件
    /// 形态，作为 member target）。gateway 注入生产
    /// `ProductionPolicyTargetResolver::for_lc`——成员三层身份与 git-dir
    /// identity 复验走真实生产路径，不用成员 fallback。
    struct SeparatedRootMemberFixture {
        _root: tempfile::TempDir,
        project_id: String,
        /// canonical LC root（含空格；非 Git）。
        lc_root: PathBuf,
        /// 成员主仓（真实 Git checkout）；当前无读取点，保留 fixture 形状。
        #[allow(dead_code)]
        member_main: PathBuf,
        /// 成员主仓的真实链接 worktree（member target）。
        member_worktree: PathBuf,
        member_id: String,
        checkout_id: String,
        streaming_adapter: Arc<CountingStreamingAdapter>,
        gateway: LogicalCodebaseProviderGateway,
    }

    fn separated_root_member_fixture() -> SeparatedRootMemberFixture {
        let root = tempfile::tempdir().expect("temporary product root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let project = ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "separated-form-project".to_string(),
                description: None,
            })
            .expect("create project");

        // 非 Git LC root（含空格）+ symlink alias（登记/manifest 投影字面，
        // 权威判据 canonical 相等）。
        let lc_root = root.path().join("lc root");
        std::fs::create_dir_all(&lc_root).expect("create lc root");
        let lc_root_alias = root.path().join("lc-root-alias");
        std::os::unix::fs::symlink(&lc_root, &lc_root_alias).expect("symlink lc root alias");

        // 成员主仓：真实 Git checkout；再挂真实链接 worktree（.git 文件）。
        let member_main = root.path().join("member main");
        std::fs::create_dir_all(&member_main).expect("create member main");
        run_git(&member_main, &["init", "--quiet", "-b", "main"]);
        run_git(&member_main, &["config", "user.email", "member@example.test"]);
        run_git(&member_main, &["config", "user.name", "Separated Form Member"]);
        std::fs::write(member_main.join("README.md"), "# member\n").expect("write member file");
        run_git(&member_main, &["add", "README.md"]);
        run_git(&member_main, &["commit", "--quiet", "-m", "initial commit"]);
        let member_worktree = root.path().join("member wt");
        run_git(
            &member_main,
            &[
                "worktree",
                "add",
                "--quiet",
                member_worktree.to_str().expect("utf-8 member worktree path"),
                "-b",
                "wt_0001",
            ],
        );

        // LC 子树权威记录（member/checkout/identity registry 三层身份）。
        let record = LogicalCodebaseStore::new(paths.clone())
            .create(
                &project.id,
                LogicalCodebaseCreateInput {
                    name: "separated-lc".to_string(),
                    aggregate_root: lc_root_alias.clone(),
                },
            )
            .expect("create lc record");
        let lc_id = record.id;
        let authority = LogicalCodebaseStore::for_lc(paths.clone(), &lc_id);
        let member_id = LogicalRepositoryId(Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        let manifest = LogicalCodebaseManifest::new(&project.id, lc_root_alias, vec![member_id]);
        authority
            .save_manifest(&project.id, &manifest)
            .expect("save lc manifest");
        let now = "2026-10-02T00:00:00Z".to_string();
        let canonical_member_main =
            std::fs::canonicalize(&member_main).expect("canonical member main");
        let physical_repository_id = format!("repository_{}", Uuid::new_v4().simple());
        let source_identity =
            RepositorySourceIdentity::from_git_parts(&member_main, member_main.join(".git"), None);
        authority
            .save_member(
                &project.id,
                &CodebaseMemberRecord {
                    logical_repository_id: member_id,
                    physical_repository_id: physical_repository_id.clone(),
                    alias: "member".to_string(),
                    role: "repository".to_string(),
                    ordinal: 0,
                    source_identity: source_identity.clone(),
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
            .expect("save lc member");
        authority
            .save_checkout(
                &project.id,
                &RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id: member_id,
                    physical_repository_id: physical_repository_id.clone(),
                    kind: CheckoutKind::Main,
                    canonical_path: canonical_member_main,
                    checkout_path_hash: "sha256:separated-checkout".to_string(),
                    git_dir_identity: source_identity.git_dir_identity(),
                    revision: None,
                    availability: CheckoutAvailability::Available,
                    observed_at: now.clone(),
                    created_at: now.clone(),
                    updated_at: now,
                },
            )
            .expect("save lc checkout");
        IdentityRegistryStore::new(paths.clone())
            .upsert_active(
                &project.id,
                IdentityRegistryEntry::active(
                    source_identity,
                    member_id,
                    physical_repository_id,
                    checkout_id,
                    "separated-form-fixture".to_string(),
                ),
            )
            .expect("register identity");

        let policy_store = AggregatePolicyArtifactStore::for_lc(paths.clone(), &lc_id);
        policy_store
            .ensure_bootstrap(&manifest)
            .expect("bootstrap policy");

        let streaming_adapter = Arc::new(CountingStreamingAdapter::new());
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderName::ClaudeCode, streaming_adapter.clone());
        let gateway = LogicalCodebaseProviderGateway::with_audit(
            policy_store,
            Arc::new(StaticCapabilitySource::new("1.4.0")),
            Arc::new(ProductionPolicyTargetResolver::for_lc(paths.clone(), &lc_id)),
            Arc::new(registry),
            Arc::new(StubSyncAdapter),
            always_available_gate(),
            Arc::new(GatewayRunAudit::new()),
            std::fs::canonicalize(&lc_root).expect("canonical authority root"),
        );

        SeparatedRootMemberFixture {
            _root: root,
            project_id: project.id,
            lc_root,
            member_main,
            member_worktree,
            member_id: member_id.0.to_string(),
            checkout_id: checkout_id.0.to_string(),
            streaming_adapter,
            gateway,
        }
    }

    impl SeparatedRootMemberFixture {
        /// planning 只读请求：cwd=canonical LC root（非 Git，≠member target）。
        fn separated_planning_request(&self) -> SessionLaunchRequest {
            SessionLaunchRequest {
                project_id: self.project_id.clone(),
                provider: ProviderRef::claude_code("cap_claude_1_4_0"),
                action: SessionPolicyAction::PlanningReadOnly,
                target: PolicyTarget::checkout(
                    self.member_id.clone(),
                    self.checkout_id.clone(),
                    self.member_worktree.clone(),
                ),
                working_directory: self.lc_root.clone(),
                readable_roots: vec![self.lc_root.clone()],
                writable_roots: Vec::new(),
                config_artifact_ref: "sha256:managed-config-artifact".to_string(),
            }
        }
    }

    /// streaming 探针 input（cwd 显式传 working_dir；与主文件
    /// GatewayFixture::streaming_input 同型）。
    fn streaming_probe(
        working_dir: PathBuf,
        resume_id: Option<String>,
    ) -> crate::cross_cutting::streaming_provider::StreamingProviderInput {
        use crate::cross_cutting::streaming_provider::{
            ProviderPermissionMode, StreamingProviderInput,
        };
        use crate::protocol::contracts::{AdapterRole, ProviderType};
        StreamingProviderInput {
            working_directory: None,
            baseline_tree: None,
            tool_policy: None,
            audit_sink: None,
            provider_type: ProviderType::ClaudeCode,
            role: AdapterRole::Executor,
            prompt: "probe".to_string(),
            working_dir,
            workspace_session_id: None,
            resume_provider_session_id: resume_id,
            permission_mode: ProviderPermissionMode::Auto,
            structured_output_contract: None,
            env_vars: Default::default(),
            timeout_secs: 1,
        }
    }

    /// Task 3.1（REQ-ENV-10/11）：合法 cwd≠target 分离形态回归锁——root cwd
    ///（非 Git、路径含空格、经 symlink alias 登记）+ member target（真实
    /// Git 链接 worktree）经生产 resolver 全链放行：validate 冻结 canonical
    /// cwd 与 canonical target，spawn 复验后真实启动；spawn input cwd 回退
    /// member target 则 spawn 前 fail-closed（禁止回退 member cwd）。
    #[test]
    fn logical_root_cwd_member_target_fixture_is_allowed() {
        let fixture = separated_root_member_fixture();
        let canonical_root =
            std::fs::canonicalize(&fixture.lc_root).expect("canonical lc root");
        let canonical_worktree = std::fs::canonicalize(&fixture.member_worktree)
            .expect("canonical member worktree");

        // (a) validate 放行：envelope 冻结 canonical cwd（root≠target）与经
        // 生产 resolver 复验（三层身份 + 真实 git-dir identity：.git 文件
        // 指向成员主仓 .git/worktrees）的 canonical member target。
        let validated = fixture
            .gateway
            .validate(fixture.separated_planning_request())
            .expect("root cwd with real git member target must validate");
        assert_eq!(validated.envelope().working_directory, canonical_root);
        assert_eq!(validated.envelope().target.worktree, canonical_worktree);
        assert_ne!(
            validated.envelope().working_directory,
            validated.envelope().target.worktree
        );

        // spawn 放行：input cwd=冻结 root → 真实启动（合法分离形态）。
        let launch = ValidatedStreamingProviderInput::new(
            streaming_probe(fixture.lc_root.clone(), None),
            validated,
        );
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime
            .block_on(
                fixture
                    .gateway
                    .start_streaming(launch, CancellationToken::new()),
            )
            .expect("legal separated form must spawn through real git identity revalidation");
        assert_eq!(fixture.streaming_adapter.start_count(), 1);

        // (b) spawn input cwd 回退 member target（≠冻结 root cwd）→ spawn 前
        // fail-closed，零新增启动。
        let validated = fixture
            .gateway
            .validate(fixture.separated_planning_request())
            .expect("second validate for cwd fallback probe");
        let launch = ValidatedStreamingProviderInput::new(
            streaming_probe(fixture.member_worktree.clone(), None),
            validated,
        );
        let error = match runtime.block_on(
            fixture
                .gateway
                .start_streaming(launch, CancellationToken::new()),
        ) {
            Err(error) => error,
            Ok(_) => panic!("member cwd fallback must fail closed"),
        };
        assert!(
            matches!(&error, ProviderGatewayError::TargetMismatch { field } if field == "cwd"),
            "expected cwd mismatch, got {error:?}"
        );
        assert_eq!(fixture.streaming_adapter.start_count(), 1);
    }

    /// Task 3.1（REQ-ENV-10/11）：分离形态下 resume 指纹的 root/target 两
    /// 维度漂移都拒绝续接——仅 root cwd 漂移（target 不变）或仅 member
    /// target 漂移（cwd 不变）均 supersede 旧会话并 StartNew（审计记
    /// resume_fingerprint_mismatch）；全维度一致才 Resume，全程零 spawn。
    #[test]
    fn logical_root_or_target_fingerprint_drift_is_rejected() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let project_id = fixture.manifest().project_id;
        let authority = fixture.paths.root().to_path_buf();
        let member_a = authority.join("member wt a");
        let member_b = authority.join("member wt b");
        // 旧会话的 root cwd 字面（authority 内另一目录）：与当前 root 恰只
        // 差 cwd 一个维度，用于隔离指纹的 root 维度。
        let legacy_root = authority.join("legacy root");
        for dir in [&member_a, &member_b, &legacy_root] {
            std::fs::create_dir_all(dir).expect("create fixture dir");
        }
        let gateway = fixture.gateway();

        // 旧会话 A：cwd=legacy root、target=member a。
        let legacy_fingerprint = gateway
            .validate(planning_request_with_cwd(
                &project_id,
                legacy_root.clone(),
                member_a.clone(),
                authority.clone(),
            ))
            .expect("validate legacy root session")
            .fingerprint()
            .clone();

        // 当前会话：cwd=authority root（root 迁移后形态），target 不变。
        let current = planning_request_with_cwd(
            &project_id,
            authority.clone(),
            member_a.clone(),
            authority.clone(),
        );

        // 仅 root cwd 维度漂移 → 拒绝 resume：supersede 旧会话并 StartNew。
        match gateway
            .resume_or_start(ResumeSessionLaunchRequest {
                launch: current.clone(),
                previous_fingerprint: legacy_fingerprint,
                previous_session_id: "sess_root_drift".to_string(),
            })
            .expect("root drift resume decision")
        {
            GatewaySessionDisposition::StartNew {
                superseded_session_id,
                ..
            } => {
                assert_eq!(superseded_session_id, "sess_root_drift");
            }
            other => panic!("root cwd drift must supersede, got {other:?}"),
        }
        assert_eq!(fixture.gateway_audit().supersede_count(), 1);
        assert!(fixture
            .gateway_audit()
            .last_supersede_reason()
            .is_some_and(|reason| reason == "resume_fingerprint_mismatch"));

        // 仅 target 维度漂移（cwd 与当前一致，旧会话 target=member b）→
        // 拒绝 resume。
        let member_b_fingerprint = gateway
            .validate(planning_request_with_cwd(
                &project_id,
                authority.clone(),
                member_b.clone(),
                authority.clone(),
            ))
            .expect("validate member b session")
            .fingerprint()
            .clone();
        match gateway
            .resume_or_start(ResumeSessionLaunchRequest {
                launch: current.clone(),
                previous_fingerprint: member_b_fingerprint,
                previous_session_id: "sess_target_drift".to_string(),
            })
            .expect("target drift resume decision")
        {
            GatewaySessionDisposition::StartNew {
                superseded_session_id,
                ..
            } => {
                assert_eq!(superseded_session_id, "sess_target_drift");
            }
            other => panic!("member target drift must supersede, got {other:?}"),
        }
        assert_eq!(fixture.gateway_audit().supersede_count(), 2);

        // 对照：root cwd 与 target 全维度一致 → Resume，零新增 supersede。
        let current_fingerprint = gateway
            .validate(current.clone())
            .expect("validate current session")
            .fingerprint()
            .clone();
        let disposition = gateway
            .resume_or_start(ResumeSessionLaunchRequest {
                launch: current,
                previous_fingerprint: current_fingerprint,
                previous_session_id: "sess_current".to_string(),
            })
            .expect("consistent resume decision");
        assert!(matches!(disposition, GatewaySessionDisposition::Resume(_)));
        assert_eq!(fixture.gateway_audit().supersede_count(), 2);

        // resume 判定全程零 spawn（拒绝续接 ≠ 启动新 provider）。
        assert_eq!(fixture.registry_start_count(), 0);
    }
