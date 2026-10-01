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
