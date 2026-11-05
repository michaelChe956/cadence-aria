/// Task 2.8(REQ-PLN-03,planning snapshot 贯穿):StaleContext 重建(B3)把
/// input worktree 换成 rebuilt.cwd(聚合根)——`start_work_item_plan_author`
/// 必须把 input 的独立 cwd 重绑 envelope 冻结的 root,spawn 前复验不得因
/// 「input root vs envelope 成员 cwd」漂移而拒绝重建 run;worktree 字段
/// 原样透传(target 语义)。
#[tokio::test]
async fn start_work_item_plan_author_rebinds_input_cwd_to_envelope_root_for_rebuild() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let fixture = t14_logical_fixture_with_gateway_extras(Arc::new(CapturingStreamingAdapter {
        inputs: input_tx,
        start_mark: None,
        observed_start_mark: Arc::default(),
    }));
    let canonical_root = t14_canonical_root(&fixture);
    let launch = {
        let engine = fixture.engine.lock().await;
        resolve_plan_author_launch(&engine).expect("resolve logical launch")
    };

    // rebuild 形态:worktree=rebuilt.cwd=聚合根(provider_run.rs B3 分支),
    // working_directory 未注入(None → 旧字段回填 worktree)。
    let input = streaming_input(canonical_root.clone());
    let session = start_work_item_plan_author(
        launch,
        Arc::new(CapturingStreamingAdapter {
            inputs: mpsc::unbounded_channel().0,
            start_mark: None,
            observed_start_mark: Arc::default(),
        }),
        input,
        CancellationToken::new(),
        None,
    )
    .await;

    assert!(
        session.is_ok(),
        "rebuild 形态启动不得被 cwd 漂移拒绝: {:?}",
        session.as_ref().err()
    );
    let captured = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("capture adapter input")
        .expect("captured input");
    assert_eq!(
        captured.working_directory.as_deref(),
        Some(canonical_root.as_path()),
        "spawn cwd 必须重绑 envelope 冻结的 canonical root,got: {:?}",
        captured.working_directory
    );
    assert_eq!(
        captured.working_dir, canonical_root,
        "rebuild worktree(rebuilt.cwd)原样透传(target 语义)"
    );
}

/// Task 2.8(REQ-PLN-01/07):正常(非重建)plan author input 的 worktree 是
/// 成员 checkout——`start_work_item_plan_author` 同样必须把独立 cwd 冻结到
/// envelope root,provider 不得以成员 checkout 为 cwd 启动。
#[tokio::test]
async fn start_work_item_plan_author_binds_normal_input_cwd_to_envelope_root() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let fixture = t14_logical_fixture_with_gateway_extras(Arc::new(CapturingStreamingAdapter {
        inputs: input_tx,
        start_mark: None,
        observed_start_mark: Arc::default(),
    }));
    let canonical_root = t14_canonical_root(&fixture);
    let member = fixture.member_checkout_root();
    let launch = {
        let engine = fixture.engine.lock().await;
        resolve_plan_author_launch(&engine).expect("resolve logical launch")
    };

    let input = streaming_input(member.clone());
    let session = start_work_item_plan_author(
        launch,
        Arc::new(CapturingStreamingAdapter {
            inputs: mpsc::unbounded_channel().0,
            start_mark: None,
            observed_start_mark: Arc::default(),
        }),
        input,
        CancellationToken::new(),
        None,
    )
    .await;

    assert!(
        session.is_ok(),
        "normal 形态启动失败: {:?}",
        session.as_ref().err()
    );
    let captured = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("capture adapter input")
        .expect("captured input");
    assert_eq!(
        captured.working_directory.as_deref(),
        Some(canonical_root.as_path()),
        "spawn cwd 必须冻结 canonical root(非成员 checkout),got: {:?}",
        captured.working_directory
    );
    assert_eq!(
        captured.working_dir, member,
        "正常形态 worktree 保持成员 checkout"
    );
}

/// A1-b1(Bug45 批2 环境回退):plan/split 唯一物理启动口停止注入环境——
/// caller 的 env_vars(sentinel 覆盖层)原样透传;宿主 PATH 等由底座
/// (process_manager)继承到达子进程,裸名 CLI(kimi)解析不断链。
#[tokio::test]
async fn start_work_item_plan_author_preserves_caller_env_without_injection() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let fixture = t14_logical_fixture_with_gateway_extras(Arc::new(CapturingStreamingAdapter {
        inputs: input_tx,
        start_mark: None,
        observed_start_mark: Arc::default(),
    }));
    let canonical_root = t14_canonical_root(&fixture);
    let launch = {
        let engine = fixture.engine.lock().await;
        resolve_plan_author_launch(&engine).expect("resolve logical launch")
    };

    let mut input = streaming_input(canonical_root.clone());
    input.env_vars.insert(
        "ARIA_A1B_SENTINEL".to_string(),
        "caller-override".to_string(),
    );
    let session = start_work_item_plan_author(
        launch,
        Arc::new(CapturingStreamingAdapter {
            inputs: mpsc::unbounded_channel().0,
            start_mark: None,
            observed_start_mark: Arc::default(),
        }),
        input,
        CancellationToken::new(),
        None,
    )
    .await;

    assert!(
        session.is_ok(),
        "plan author 启动不得因环境构造失败: {:?}",
        session.as_ref().err()
    );
    let captured = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("capture adapter input")
        .expect("captured input");
    assert_eq!(
        captured
            .env_vars
            .get("ARIA_A1B_SENTINEL")
            .map(String::as_str),
        Some("caller-override"),
        "plan author 启动必须原样透传 caller 显式覆盖层"
    );
    assert!(
        !captured.env_vars.contains_key("PATH") && !captured.env_vars.contains_key("HOME"),
        "plan author 启动不得再注入白名单 PATH/HOME(宿主值由底座继承): {:?}",
        captured.env_vars
    );
}

fn workspace_engine(fixture: &GatewayFixture, with_gateway: bool) -> WorkspaceEngine {
    workspace_engine_with_author(fixture, with_gateway, ProviderName::ClaudeCode)
}

/// C-2:构造指定 author provider 的 engine(其余与 `workspace_engine` 一致),
/// 供 provider 身份透传/失败关闭测试驱动 session 配置。
fn workspace_engine_with_author(
    fixture: &GatewayFixture,
    with_gateway: bool,
    author: ProviderName,
) -> WorkspaceEngine {
    let (event_tx, _event_rx) = mpsc::channel::<crate::product::workspace_engine::EngineEvent>(8);
    let mut session = workspace_session(fixture.aggregate_root.clone());
    session.author_provider = author;
    let engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(
            fixture.paths.root().join("checkpoints"),
        )),
        event_tx,
        session,
    );
    if with_gateway {
        engine.with_logical_provider_gateway(fixture.gateway.clone())
    } else {
        engine
    }
}

#[tokio::test]
async fn start_work_item_plan_author_routes_logical_through_gateway_and_records_audit() {
    let fixture = t14_logical_fixture_with_gateway_extras(Arc::new(CountingStreamingAdapter {
        starts: Arc::new(AtomicUsize::new(0)),
    }));
    let audit = {
        let engine = fixture.engine.lock().await;
        engine
            .logical_provider_gateway()
            .expect("fixture gateway")
            .audit()
    };
    assert_eq!(audit.stream_launches(), 0);

    let launch = {
        let engine = fixture.engine.lock().await;
        resolve_plan_author_launch(&engine).expect("resolve logical launch")
    };
    let input = streaming_input(t14_canonical_root(&fixture));

    let session = start_work_item_plan_author(
        launch,
        Arc::new(CountingStreamingAdapter {
            starts: Arc::new(AtomicUsize::new(0)),
        }),
        input,
        CancellationToken::new(),
        None,
    )
    .await;

    assert!(
        session.is_ok(),
        "gateway launch failed: {:?}",
        session.as_ref().err()
    );
    assert_eq!(audit.stream_launches(), 1);
}

/// C-2(组1):logical author 配置 Claude 时,launch 必须携带 session 配置的
/// author provider 身份,validated envelope 冻结 ClaudeCode dialect——不得由
/// 启动点硬编码推定。
#[tokio::test]
async fn logical_plan_launch_carries_session_author_provider_identity() {
    let fixture = t14_logical_fixture_with_gateway_extras(Arc::new(CountingStreamingAdapter {
        starts: Arc::new(AtomicUsize::new(0)),
    }));

    let plan_launch = {
        let engine = fixture.engine.lock().await;
        resolve_plan_author_launch(&engine).expect("claude author must resolve a logical launch")
    };
    let PlanAuthorLaunch::Logical(plan) = &plan_launch else {
        panic!("logical session must resolve a Logical launch");
    };
    assert_eq!(
        plan.launch.author_provider,
        ProviderName::ClaudeCode,
        "launch must carry the session-configured author provider"
    );
    assert_eq!(
        plan.validated.envelope().provider_dialect,
        ProviderDialect::ClaudeCodeCliV1,
        "validated envelope must freeze the ClaudeCode dialect"
    );
}

/// Task 1a 后合同重钉(6b carry):Pi/Kimi 是合法显式映射,「静默以 Claude
/// 跑完整 planning run」的防线从集中映射移到 capability 准入——fixture 未
/// 给 Pi/Kimi 落 durable capability 记录,logical 启动必须在 launch 分格门
/// 显式失败(判别码 + provider 名),gateway 零启动。
#[tokio::test]
async fn logical_plan_launch_fails_closed_for_unsupported_author_provider() {
    for author in [ProviderName::Pi, ProviderName::KimiCode] {
        let fixture =
            super::single_candidate_provider_run::ProviderRunFixture::new_logical_with_gateway_extras(
                true,
                Arc::new(CountingStreamingAdapter {
                    starts: Arc::new(AtomicUsize::new(0)),
                }),
                super::single_candidate_lc_admission::AliasLcGatewayExtras {
                    capability: Some(Arc::new(T14ClaudeOnlyCapabilitySource)),
                    ..super::single_candidate_lc_admission::AliasLcGatewayExtras::default()
                },
            );
        let (audit, error) = {
            let mut engine = fixture.engine.lock().await;
            engine.session.author_provider = author.clone();
            (
                engine
                    .logical_provider_gateway()
                    .expect("fixture gateway")
                    .audit(),
                resolve_plan_author_launch(&engine)
                    .err()
                    .unwrap_or_else(|| {
                        panic!("{author:?} author must not silently launch as Claude")
                    }),
            )
        };
        assert!(
            error
                .details
                .contains("provider_capability_launch_not_confirmed"),
            "expected provider_capability_launch_not_confirmed, got: {}",
            error.details
        );
        assert!(
            error.details.contains(&format!("{author:?}")),
            "error must name the configured author provider, got: {}",
            error.details
        );
        assert_eq!(audit.stream_launches(), 0, "no gateway session may start");
    }
}

/// r47 修订:author 配置 Codex 时,logical 启动不再命中路由级全阻(Task 13
/// 期形态已移除——LC 投影后 Codex 恒非 danger);改钉新契约:capability 行
/// 未探针签发(bootstrap Unknown)时 fail-closed 为
/// provider_capability_launch_not_confirmed,且不静默改 Claude 启动。
#[tokio::test]
async fn logical_plan_launch_codex_author_hits_req_env05_route_block() {
    let fixture =
        super::single_candidate_provider_run::ProviderRunFixture::new_logical_with_gateway_extras(
            true,
            Arc::new(CountingStreamingAdapter {
                starts: Arc::new(AtomicUsize::new(0)),
            }),
            super::single_candidate_lc_admission::AliasLcGatewayExtras {
                capability: Some(Arc::new(T14ClaudeOnlyCapabilitySource)),
                ..super::single_candidate_lc_admission::AliasLcGatewayExtras::default()
            },
        );
    let (audit, error) = {
        let mut engine = fixture.engine.lock().await;
        engine.session.author_provider = ProviderName::Codex;
        (
            engine
                .logical_provider_gateway()
                .expect("fixture gateway")
                .audit(),
            resolve_plan_author_launch(&engine)
                .err()
                .expect("codex author without probe-confirmed row must fail closed"),
        )
    };
    assert!(
        error
            .details
            .contains("provider_capability_launch_not_confirmed"),
        "expected capability-row fail-closed (route blanket block removed in r47), got: {}",
        error.details
    );
    assert_eq!(audit.stream_launches(), 0, "no gateway session may start");
}

#[tokio::test]
async fn start_work_item_plan_author_none_uses_legacy_provider_start_unchanged() {
    let fixture = gateway_fixture();
    let audit = fixture.gateway.audit();
    assert_eq!(audit.stream_launches(), 0);

    let starts = Arc::new(AtomicUsize::new(0));
    let provider: Arc<dyn StreamingProviderAdapter> = Arc::new(CountingStreamingAdapter {
        starts: starts.clone(),
    });
    let engine = workspace_engine(&fixture, false);
    let plan_launch = resolve_plan_author_launch(&engine).expect("legacy launch");
    let input = streaming_input(fixture.aggregate_root.clone());

    let session =
        start_work_item_plan_author(plan_launch, provider, input, CancellationToken::new(), None)
            .await;

    assert!(
        session.is_ok(),
        "legacy start failed: {:?}",
        session.as_ref().err()
    );
    assert_eq!(
        starts.load(Ordering::SeqCst),
        1,
        "legacy provider.start must run"
    );
    assert_eq!(
        audit.stream_launches(),
        0,
        "legacy path must not touch gateway audit"
    );
}

#[tokio::test]
async fn start_work_item_plan_author_bounds_launch_stall_at_shared_choke_point() {
    // 缺口（owner r20/r21 现场 37–40 分钟静默）：plan author 启动口在拿到
    // ProviderSession **之前**没有任何停滞界——看门狗全部在驱动循环内生效，
    // 握手期零 provider 事件、零 WS 帧，只能等整 run 总预算 3h 收口。
    // 本用例以可注入阈值（50ms，生产 300s）构造「启动永不返回」：
    // 启动口必须在该界内以稳定码失败，而不是无限等待。
    let _stall_guard = crate::product::workspace_engine::set_provider_launch_stall_timeout_for_test(
        std::time::Duration::from_millis(50),
    );

    let fixture = gateway_fixture();
    let starts = Arc::new(AtomicUsize::new(0));
    let provider: Arc<dyn StreamingProviderAdapter> = Arc::new(HangingStartStreamingAdapter {
        starts: starts.clone(),
    });
    let engine = workspace_engine(&fixture, false);
    let plan_launch = resolve_plan_author_launch(&engine).expect("legacy launch");
    let input = streaming_input(fixture.aggregate_root.clone());

    // 外层守卫只防止用例自身挂死：真正的界必须由启动口给出。
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        start_work_item_plan_author(plan_launch, provider, input, CancellationToken::new(), None),
    )
    .await
    .expect("launch stall bound must fire before the test guard");

    let error = match outcome {
        Ok(_) => panic!("hanging provider start must fail, not wait forever"),
        Err(error) => error,
    };
    assert!(
        error.details.contains("provider_launch_stall"),
        "stable reason code missing: {}",
        error.details
    );
    assert_eq!(
        error.code,
        crate::protocol::provider_errors::ProviderErrorCode::ProviderTimeout,
        "launch stall must surface as the provider timeout code"
    );
    assert_eq!(
        error.timeout_status,
        TimeoutStatus::HardTimeoutKilled,
        "launch stall must be reported as a hard timeout"
    );
    assert_eq!(
        starts.load(Ordering::SeqCst),
        1,
        "the hanging launch must have been attempted exactly once"
    );
}

#[test]
fn workspace_engine_accessors_expose_logical_launch() {
    let fixture = gateway_fixture();
    let engine = workspace_engine(&fixture, true);

    assert!(engine.logical_provider_gateway().is_some());
    assert_eq!(
        engine.logical_planning_launch(),
        Some(("project_0001".to_string(), fixture.aggregate_root.clone()))
    );
}

#[tokio::test]
async fn logical_plan_validate_failure_is_reported_by_handler() {
    let fixture = gateway_fixture();
    let repo_path = fixture._temp.path().join("member");
    std::fs::create_dir_all(&repo_path).expect("member checkout");
    let repository = RepositoryStore::new(fixture.paths.clone())
        .create(CreateRepositoryInput {
            project_id: "project_0001".to_string(),
            name: "member".to_string(),
            path: repo_path.clone(),
            default_policy_preset: None,
            default_provider_mode: None,
            idempotency_key: "logical-plan-validate-failure".to_string(),
        })
        .expect("repository");
    let issue = IssueStore::new(fixture.paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: Some(repository.id.clone()),
            // 归属必填（REQ-ROUTE-01）：夹具无 per-LC 权威记录 ⇒ issue 归属 project
            // 默认作用域（legacy 别名代码库 id）。
            logical_codebase_id: crate::product::logical_codebase::store::legacy_logical_codebase_id(
                "project_0001",
            ),
            title: "Logical plan validation".to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .expect("issue");
    let lifecycle = LifecycleStore::new(fixture.paths.clone());
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: issue.project_id.clone(),
            issue_id: issue.id.clone(),
            repository_id: repository.id.clone(),
            title: "Story".to_string(),
            aggregate_codebase: None,
        })
        .expect("story");
    let design = lifecycle
        .create_design_spec(CreateDesignSpecInput {
            project_id: issue.project_id.clone(),
            issue_id: issue.id.clone(),
            story_spec_ids: vec![story.id.clone()],
            title: "Design".to_string(),
            aggregate_codebase: None,
        })
        .expect("design");
    let plan = lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some("issue_work_item_plan_0001".to_string()),
            project_id: issue.project_id.clone(),
            issue_id: issue.id.clone(),
            source_story_spec_ids: vec![story.id],
            source_design_spec_ids: vec![design.id],
            options: IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Draft,
            work_item_ids: vec![],
            repository_profile_ref: None,
            verification_plan_ids: vec![],
            dependency_graph: vec![],
            created_from_provider_run: None,
            validator_findings: vec![],
        })
        .expect("plan");
    let session_record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: issue.project_id.clone(),
            issue_id: issue.id.clone(),
            entity_id: plan.id,
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: Some(ProviderName::Codex),

            review_rounds: 0,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("workspace session");

    // Keep the already-built gateway in memory, then remove its capability rows.  This
    // deterministically exercises the validate-failure early return without a real provider.
    // REQ-ROUTE-01 两态化：routing 无 legacy 单仓形态，上下文/anchor 链要求
    // （manifest, selection, ≥1 有效成员）成对 ⇒ 保留 manifest 并登记本仓为
    // LC 成员 + 带成员 selection（空成员会让计划上下文在 launch 前
    // fail-closed）；Logical 路由的 revision-input 还消费聚合索引 ⇒ 一并
    // 播种。validate 失败面由删除 capabilities.json（validate 首关唯一消费）
    // 确定性触发，policy 保持有效。
    let gateway = fixture.gateway.clone();
    let authority = LogicalCodebaseStore::new(fixture.paths.clone());
    let mut manifest = authority
        .load_manifest("project_0001")
        .expect("load fixture manifest")
        .expect("fixture manifest");
    let member_id = crate::product::logical_codebase::types::LogicalRepositoryId(uuid::Uuid::new_v4());
    let checkout_id =
        crate::product::logical_codebase::types::RepositoryCheckoutId(uuid::Uuid::new_v4());
    manifest.member_ids.push(member_id);
    manifest.membership_revision += 1;
    authority
        .save_manifest("project_0001", &manifest)
        .expect("add fixture member to manifest");
    let now = chrono::Utc::now().to_rfc3339();
    authority
        .save_member(
            "project_0001",
            &crate::product::logical_codebase::types::CodebaseMemberRecord {
                logical_repository_id: member_id,
                physical_repository_id: repository.id.clone(),
                alias: "member".to_string(),
                role: "member".to_string(),
                ordinal: 1,
                source_identity:
                    crate::product::logical_codebase::types::RepositorySourceIdentity::from_git_parts(
                        &repo_path,
                        repo_path.join(".git"),
                        Some("ssh://git@example.test/gateway/member.git".to_string()),
                    ),
                repo_type: crate::product::logical_codebase::types::RepositoryType::Unknown,
                tech_stack: Vec::new(),
                owner: None,
                tags: Vec::new(),
                default_ref: None,
                checkout_ids: vec![checkout_id],
                status: crate::product::logical_codebase::types::MemberStatus::Active,
                created_at: now.clone(),
                updated_at: now.clone(),
            },
        )
        .expect("save fixture member");
    authority
        .save_checkout(
            "project_0001",
            &crate::product::logical_codebase::types::RepositoryCheckoutRecord {
                checkout_id,
                logical_repository_id: member_id,
                physical_repository_id: repository.id.clone(),
                kind: crate::product::logical_codebase::types::CheckoutKind::Main,
                canonical_path: std::fs::canonicalize(&repo_path).expect("canonical member"),
                checkout_path_hash: "sha256:gateway-member-checkout".to_string(),
                git_dir_identity: "sha256:gateway-member-git-dir".to_string(),
                revision: Some("abc123".to_string()),
                availability:
                    crate::product::logical_codebase::types::CheckoutAvailability::Available,
                observed_at: now.clone(),
                created_at: now.clone(),
                updated_at: now,
            },
        )
        .expect("save fixture checkout");
    // strict 三层身份解析要求 repos.json 投影与登记的成员/checkout 对齐
    //（logical_repository_id / primary_checkout_id / identity_schema_version）。
    let mut projections = RepositoryStore::new(fixture.paths.clone())
        .list("project_0001")
        .expect("list fixture repositories");
    for record in projections
        .iter_mut()
        .filter(|record| record.id == repository.id)
    {
        record.logical_repository_id = Some(member_id);
        record.primary_checkout_id = Some(checkout_id);
        record.identity_schema_version = 1;
    }
    std::fs::write(
        fixture
            .paths
            .project_root("project_0001")
            .join("repos.json"),
        serde_json::to_vec_pretty(&projections).expect("serialize fixture projections"),
    )
    .expect("rewrite repos.json with registered projection");
    crate::product::logical_codebase::IssueCodebaseSelectionStore::new(fixture.paths.clone())
        .save(&crate::product::logical_codebase::IssueCodebaseSelection::explicit(
            "project_0001",
            &issue.id,
            vec![member_id],
            Vec::new(),
            Vec::new(),
            None,
        ))
        .expect("save issue selection");
    super::lc_fixture_support::ensure_planning_index_and_policy(&fixture.paths, "project_0001");
    // capability 记录的**机器级**落盘路径（REQ-PCR-07）：
    // 删掉它 ⇒ validate 首关确定失败（与作用域无关，触发语义不变）。
    std::fs::remove_file(
        fixture
            .paths
            .provider_capabilities_root()
            .join("capabilities.json"),
    )
    .expect("remove capability rows");

    let (engine_tx, _engine_rx) = mpsc::channel::<crate::product::workspace_engine::EngineEvent>(8);
    let mut session = WorkspaceSession::from_record(session_record.clone());
    session.stage = WorkspaceStage::PrepareContext;
    session.repository_path = Some(repo_path);
    let engine = Arc::new(Mutex::new(
        WorkspaceEngine::new_persistent(
            Arc::new(CheckpointStore::new(
                fixture.paths.root().join("checkpoints"),
            )),
            lifecycle,
            engine_tx,
            session,
        )
        .with_logical_provider_gateway(gateway),
    ));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(CountingStreamingAdapter {
            starts: Arc::new(AtomicUsize::new(0)),
        }),
    );
    let run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        engine,
        WorkspaceRunRegistry::default(),
        session_record.id.clone(),
        fixture.paths.clone(),
        session_record,
    );
    let (outbound_tx, mut outbound_rx) = mpsc::channel::<OutboundControl>(8);

    spawn_provider_run_from_handler(
        run_context,
        ProviderRunKind::WorkItemPlanLegacyAuthor,
        outbound_tx,
    )
    .await
    .expect("validation failure is reported from the async handler task");

    let OutboundControl::Text(json) =
        tokio::time::timeout(std::time::Duration::from_secs(1), outbound_rx.recv())
            .await
            .expect("handler error outbound")
            .expect("handler error message")
    else {
        panic!("expected text error outbound");
    };
    let message: WsOutMessage = serde_json::from_str(&json).expect("ws error message");
    let WsOutMessage::Error { message } = message else {
        panic!("expected error outbound, got {message:?}");
    };
    assert!(
        message.starts_with("logical plan launch failed:"),
        "unexpected handler error (expected logical plan launch failure): {message}"
    );
}
