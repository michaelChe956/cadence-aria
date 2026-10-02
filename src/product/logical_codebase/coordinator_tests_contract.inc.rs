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

    // Task 3.2（D3 重写）：本位置原为 Task 1.1 的源码 token 扫描锁
    // （`aggregate_coordinator_isolation_locked_against_single_repository_
    // persistence_and_git_finalize`——对 7 个聚合生产文件扫描
    // RepositoryPersistence/git_finalize/RepositoryRegistrationCoordinator/
    // RepositoryInitializationOperation 与 provider fallback token）。按
    // lc-root-initialization 3.2 契约「调用图/可达性断言替字符串扫描锁」
    // 移除：字符串命中不等于调用发生、未命中也不等于不可达，行为证据
    // 才是锁。替代锁（两条，均可观测调用图边界）：
    // 1. `aggregate_root_contract_does_not_reuse_repository_registration_
    //    or_git_finalize`（本文件下方）——coordinator 级行为锁：五步布局、
    //    3 次 gateway 启动全部带 policy digest、每次启动 ClaudeCode、
    //    cwd=canonical 聚合根。
    // 2. `web::handlers::aggregate_initialization::tests::
    //    shared_executor_cannot_reach_repository_registration_or_git_
    //    finalize`（tests_parts/root_safety.inc.rs）——生产接线可达性锁：
    //    共享四命令 executor 完整跑通五步 recipe 时，成员 Git 指纹/根文件
    //    清单/单仓 Repository+operation 持久化指纹零变化、启动全经 gateway
    //    审计、命令文本与冻结命令索引一致。单仓类型归属地另有
    //    `repository_store::registration::tests::
    //    aggregate_initialization_is_isolated_from_single_repository_
    //    registration_and_git_finalize` 的模块树断言。

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
