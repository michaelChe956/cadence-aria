// Task 2.1（lc-root-initialization，REQ-ENV-01/ENV-10）测试 doubles。
// 物理拆分（1200 行守卫）：经 include! 挂载，模块域与 provider_run_events.rs 相同。

/// 记录启动 input 并立即完成的 capture provider。gateway registry 与 run
/// registry 各持独立实例，用启动计数区分「经 gateway」与「直连」经路。
pub(super) struct RootCwdCaptureAuthorProvider {
    pub(super) starts: Arc<AtomicUsize>,
    pub(super) inputs: mpsc::UnboundedSender<StreamingProviderInput>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for RootCwdCaptureAuthorProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        let _ = self.inputs.send(input);
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::plain(
                    String::new(),
                    None,
                )))
                .await;
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }

    /// Task 7 分流收口:gateway 只经 validated trait 分发。
    async fn start_validated(
        &self,
        validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (input, _launch) = validated.into_parts();
        self.start(input, cancel).await
    }

    async fn run_streaming(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        unreachable!("workspace provider-run tests use start")
    }
}

/// 组装 capture provider 三件套（启动计数、input 接收端、adapter）。
#[allow(clippy::type_complexity)]
pub(super) fn root_cwd_capture_provider() -> (
    Arc<AtomicUsize>,
    mpsc::UnboundedReceiver<StreamingProviderInput>,
    Arc<RootCwdCaptureAuthorProvider>,
) {
    let starts = Arc::new(AtomicUsize::new(0));
    let (inputs_tx, inputs_rx) = mpsc::unbounded_channel();
    (
        starts.clone(),
        inputs_rx,
        Arc::new(RootCwdCaptureAuthorProvider {
            starts,
            inputs: inputs_tx,
        }),
    )
}

/// 等待下一条被 capture 的 provider input（超时即测试失败，不悬挂）。
pub(super) async fn next_captured_input(
    inputs: &mut mpsc::UnboundedReceiver<StreamingProviderInput>,
) -> StreamingProviderInput {
    tokio::time::timeout(std::time::Duration::from_secs(5), inputs.recv())
        .await
        .expect("captured provider input within timeout")
        .expect("capture channel must stay open")
}

/// Task 2.1（REQ-ENV-01/ENV-10，映射 openspec tasks 2.3 Author/ChoiceFollowup 面）：
/// LC 会话 Author 首轮与 ChoiceFollowup 两次 launch 的 cwd 都等于 canonical
/// root、target 是显式 member/checkout，follow-up 不绕过 gateway；choice 内容
/// 不构造新 target、也不降为无策略 Executor。
#[tokio::test]
async fn logical_author_and_choice_followup_keep_root_cwd_and_target_separate() {
    let (gateway_starts, mut gateway_inputs, gateway_author) = root_cwd_capture_provider();
    let fixture = ProviderRunFixture::new_logical(true, gateway_author);
    let (direct_starts, _direct_inputs, direct_sentinel) = root_cwd_capture_provider();
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, direct_sentinel);
    let (_inbound, _outbound_rx, run_context, outbound_tx) =
        sc_context_with_registry(&fixture, registry);
    // 夹具拓扑：成员 checkout 是聚合根（canonical root）的直接子目录。
    let member_root = fixture.member_checkout_root();
    let canonical_root = std::fs::canonicalize(member_root.parent().unwrap()).unwrap();
    let canonical_member = std::fs::canonicalize(&member_root).unwrap();

    spawn_provider_run_from_event(
        run_context.clone(),
        ProviderRunKind::Author {
            content: "lc author first round".to_string(),
        },
        None,
        outbound_tx.clone(),
    )
    .await
    .expect("spawn lc author run");
    let first = next_captured_input(&mut gateway_inputs).await;
    spawn_provider_run_from_event(
        run_context.clone(),
        ProviderRunKind::AuthorChoiceFollowup {
            content: "lc choice followup delta".to_string(),
        },
        None,
        outbound_tx.clone(),
    )
    .await
    .expect("spawn lc choice followup run");
    let followup = next_captured_input(&mut gateway_inputs).await;

    assert_eq!(
        direct_starts.load(Ordering::SeqCst),
        0,
        "LC author/follow-up 不得绕过 gateway 直连 provider"
    );
    assert_eq!(
        gateway_starts.load(Ordering::SeqCst),
        2,
        "首轮与 follow-up 都必须经 gateway 启动"
    );
    let deny_policy =
        crate::cross_cutting::streaming_provider::ProviderToolPolicy::deny_file_write_builtins();
    for input in [&first, &followup] {
        assert_eq!(
            input.working_directory.as_deref(),
            Some(canonical_root.as_path()),
            "LC 会话 cwd 必须是 canonical root"
        );
        assert_eq!(
            input.working_dir, canonical_member,
            "target worktree 保持成员 checkout（cwd/target 分离）"
        );
        assert_eq!(input.tool_policy, Some(deny_policy.clone()), "策略未放宽");
    }
    assert!(
        followup.prompt.contains("lc choice followup delta"),
        "follow-up 以 choice 内容原样驱动（DeltaOnly），不构造新 target"
    );

    // envelope 面：launch 冻结显式 member/checkout target 与 root cwd；两次解析
    // 的 resume fingerprint 相等（首轮与 follow-up 复用同一 launch/resume 面）。
    let first_launch = fixture
        .engine
        .lock()
        .await
        .resolve_author_root_launch(&fixture.record)
        .expect("LC 会话必须解析出 root launch")
        .expect("gateway validate 必须通过");
    let second_launch = fixture
        .engine
        .lock()
        .await
        .resolve_author_root_launch(&fixture.record)
        .expect("LC 会话必须解析出 root launch")
        .expect("gateway validate 必须通过");
    let envelope = first_launch.envelope();
    assert_eq!(envelope.working_directory, canonical_root);
    assert_eq!(envelope.target.worktree, canonical_member);
    let lc_store = crate::product::logical_codebase::LogicalCodebaseStore::for_lc(
        fixture.app_paths.clone(),
        crate::product::logical_codebase::store::legacy_logical_codebase_id("project_0001"),
    );
    let member = &lc_store
        .list_members("project_0001")
        .expect("list lc members")[0];
    assert_eq!(
        envelope.target.logical_repository_id,
        member.logical_repository_id.0.to_string(),
        "target 是显式 member"
    );
    assert!(
        !envelope.target.checkout_id.is_empty(),
        "target 是显式 checkout"
    );
    assert_eq!(
        first_launch.fingerprint(),
        second_launch.fingerprint(),
        "同会话两次解析的 envelope/resume 面必须一致"
    );

    let _ = run_context.manager.abort_active_run().await;
}

/// Task 2.1 回归锁：单仓（无 gateway 注入）Author/ChoiceFollowup 直连路径的
/// cwd、工具策略与 DeltaOnly 字节保持原状（Legacy 零变化）。
#[tokio::test]
async fn legacy_author_choice_followup_bytes_and_cwd_are_unchanged() {
    let (direct_starts, mut direct_inputs, capture) = root_cwd_capture_provider();
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::Legacy);
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, capture);
    let (_inbound, _outbound_rx, run_context, outbound_tx) =
        sc_context_with_registry(&fixture, registry);
    let member_root = fixture.member_checkout_root();
    let canonical_member = std::fs::canonicalize(&member_root).unwrap();

    spawn_provider_run_from_event(
        run_context.clone(),
        ProviderRunKind::Author {
            content: "legacy author first round".to_string(),
        },
        None,
        outbound_tx.clone(),
    )
    .await
    .expect("spawn legacy author run");
    let first = next_captured_input(&mut direct_inputs).await;
    spawn_provider_run_from_event(
        run_context.clone(),
        ProviderRunKind::AuthorChoiceFollowup {
            content: "legacy choice followup delta".to_string(),
        },
        None,
        outbound_tx.clone(),
    )
    .await
    .expect("spawn legacy choice followup run");
    let followup = next_captured_input(&mut direct_inputs).await;

    assert_eq!(
        direct_starts.load(Ordering::SeqCst),
        2,
        "单仓两轮都保持直接 provider.start（不经 gateway）"
    );
    let deny_policy =
        crate::cross_cutting::streaming_provider::ProviderToolPolicy::deny_file_write_builtins();
    for input in [&first, &followup] {
        assert_eq!(
            input.working_directory, None,
            "单仓不注入独立 cwd（effective 回填 working_dir，行为不变）"
        );
        assert_eq!(
            input.working_dir, canonical_member,
            "单仓 cwd 保持 session.repository_path"
        );
        assert_eq!(input.tool_policy, Some(deny_policy.clone()), "策略不变");
    }
    assert!(
        followup.prompt.contains("legacy choice followup delta"),
        "DeltaOnly 追加内容原样透传"
    );

    let _ = run_context.manager.abort_active_run().await;
}
