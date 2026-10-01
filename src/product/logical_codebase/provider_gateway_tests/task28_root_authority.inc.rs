// Task 2.8（映射 tasks.md 2.2）：双工厂 root assertion 与 cwd authority 的
// 测试体（由 provider_gateway_tests.rs 的 task28_gateway_root_authority
// 模块 include，保持主文件低于 large_file_guard 1200 行上限）。
    use super::*;
    use crate::product::logical_codebase::store::LogicalCodebaseRecord;
    use crate::product::logical_codebase::{
        LogicalCodebaseStore, RootRecipeReceipt, assert_canonical_lc_root_consistent,
    };
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
