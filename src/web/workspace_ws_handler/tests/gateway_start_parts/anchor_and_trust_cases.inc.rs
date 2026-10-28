/// Task 2.8/REQ-PLN-07 + B2b1(REQ-PLN-08):planning author launch 的 cwd
/// 必须是 gateway 冻结的 canonical 聚合根;envelope target 同锚聚合根——
/// native anchor 不从 business working_dir/成员身份推导(checkout 身份为
/// 空),成员 checkout 只作 business 材料面。
#[test]
fn logical_plan_launch_uses_root_cwd_not_member_repository_path() {
    let fixture = t14_logical_fixture_with_gateway_extras(Arc::new(CountingStreamingAdapter {
        starts: Arc::new(AtomicUsize::new(0)),
    }));
    let canonical_root = t14_canonical_root(&fixture);

    let launch = {
        let engine = fixture.engine.blocking_lock();
        resolve_plan_author_launch(&engine).expect("resolve logical launch")
    };
    let PlanAuthorLaunch::Logical(plan) = &launch else {
        panic!("logical session must resolve a Logical launch");
    };
    assert_eq!(
        plan.launch.working_dir, canonical_root,
        "cwd 必须是 gateway 冻结的 canonical 聚合根"
    );
    let request = plan.launch.planning_request().expect("planning request");
    assert_eq!(
        request.working_directory, canonical_root,
        "envelope cwd 冻结 root"
    );
    assert_eq!(
        request.target.worktree, canonical_root,
        "B2b1:envelope target 恒聚合根锚(REQ-PLN-08 native anchor)"
    );
    assert!(
        request.target.checkout_id.is_empty(),
        "target 不携带成员 checkout 身份"
    );
    assert!(
        request.target.logical_repository_id.is_empty(),
        "target 不携带成员 logical 身份"
    );
    assert_eq!(
        request.readable_roots,
        vec![canonical_root],
        "readable roots=root"
    );
    assert!(request.writable_roots.is_empty(), "planning 只读写根为空");
}

// ---------------------------------------------------------------------------
// Task 1b 段②(lcg_t01):WS streaming Plan/split 的 run handle 收口与
// 全入口 projection/sink 绑定。
// ---------------------------------------------------------------------------

/// 段②(lcg_t01):WS plan/split caller 依次 begin handle、bind sink、start、
/// parse、complete/fail——成功收口 completed、失败收口 failed;provider_start
/// (adapter 首行审计)先于 split run completed 落盘。
#[tokio::test]
async fn lcg_t01_ws_plan_split_run_handle_closes_on_success_and_failure() {
    use crate::product::models::ProviderName;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // T1B-P3-1:时序断言改由 adapter 真实观测——启动标记在 registry 内
    // adapter 的 `start_validated` 路径写入共享计数器并快照,收口标记在
    // complete 落盘后取下一值;若 prepared 分流未触达 adapter,快照缺失
    // 即失败(不再是测试线程手工 fetch_add 的恒真式)。
    let seq = Arc::new(AtomicUsize::new(0));
    let probe = Arc::new(std::sync::Mutex::new(None));

    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let fixture = t14_logical_fixture_with_gateway_extras(Arc::new(CapturingStreamingAdapter {
        inputs: input_tx,
        start_mark: Some(seq.clone()),
        observed_start_mark: probe.clone(),
    }));
    let plan_launch = {
        let engine = fixture.engine.lock().await;
        resolve_plan_author_launch(&engine).expect("resolve logical launch")
    };

    let lifecycle = fixture.lifecycle.clone();
    let run_ctx = super::super::run::begin_plan_split_run(
        &lifecycle,
        &fixture.record.project_id,
        &fixture.record.issue_id,
        &ProviderName::ClaudeCode,
        &fixture.record.id,
    )
    .expect("begin plan split run");

    let input = streaming_input(fixture.member_checkout_root());
    let session = start_work_item_plan_author(
        plan_launch,
        Arc::new(CapturingStreamingAdapter {
            inputs: mpsc::unbounded_channel().0,
            start_mark: None,
            observed_start_mark: Arc::default(),
        }),
        input,
        CancellationToken::new(),
        Some(&run_ctx),
    )
    .await
    .expect("logical plan author start with run handle");

    // prepare 前已绑定 run-bound sink + workspace 会话身份。
    let captured = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("capture adapter input")
        .expect("captured input");
    assert!(
        captured.audit_sink.is_some(),
        "LC 角色必须在 prepare 前绑定 run-bound audit sink"
    );
    assert_eq!(
        captured.workspace_session_id,
        Some(fixture.record.id.clone())
    );

    // provider_start(adapter 在 start_validated 路径写入的真实启动标记)
    // 先于 split run completed(complete 落盘后取下一标记)——T1B-P3-1。
    let provider_start_seq = probe
        .lock()
        .expect("start mark probe")
        .expect("adapter must have started via start_validated dispatch");
    let structured = serde_json::json!({"work_items": []});
    run_ctx
        .complete("plan prompt", &structured)
        .expect("complete consumes the plan split run handle");
    let split_completed_seq = seq.fetch_add(1, Ordering::SeqCst);
    assert!(
        provider_start_seq < split_completed_seq,
        "provider 启动标记({provider_start_seq})必须先于 split completed 标记({split_completed_seq})"
    );
    drop(session);

    // 失败路径:fail 收口 status=failed。
    let fail_ctx = super::super::run::begin_plan_split_run(
        &lifecycle,
        &fixture.record.project_id,
        &fixture.record.issue_id,
        &ProviderName::ClaudeCode,
        &fixture.record.id,
    )
    .expect("begin failing plan split run");
    fail_ctx
        .fail("provider failed before completion")
        .expect("fail closes the plan split run handle");
    let failed_run = lifecycle
        .read_work_item_split_provider_run_status(&fail_ctx.handle)
        .expect("read back failed plan split run");
    assert!(failed_run.status == "failed");
}

/// 段②(lcg_t01):所有 LC 入口经 gateway projection——Logical 启动只命中
/// registry 内 validated streaming 分发(cwd=root、target=成员、sink 已绑),
/// 直连 provider.start 计数为 0。
#[tokio::test]
async fn lcg_t01_all_lc_entrypoints_require_projection() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let fixture = t14_logical_fixture_with_gateway_extras(Arc::new(CapturingStreamingAdapter {
        inputs: input_tx,
        start_mark: None,
        observed_start_mark: Arc::default(),
    }));
    let plan_launch = {
        let engine = fixture.engine.lock().await;
        resolve_plan_author_launch(&engine).expect("resolve logical launch")
    };
    let expected_provider = crate::product::logical_codebase::ProviderRefType::ClaudeCode;
    let mapped_ref = match &plan_launch {
        super::super::run::PlanAuthorLaunch::Logical(plan) => {
            plan.launch.planning_request().expect("request").provider
        }
        _ => panic!("logical session must resolve a gateway launch"),
    };

    let direct_calls = Arc::new(AtomicUsize::new(0));
    let direct_provider: Arc<dyn StreamingProviderAdapter> = Arc::new(CountingStreamingAdapter {
        starts: direct_calls.clone(),
    });
    let member_worktree = fixture.member_checkout_root();
    let input = streaming_input(member_worktree.clone());

    let lifecycle = fixture.lifecycle.clone();
    let run_ctx = super::super::run::begin_plan_split_run(
        &lifecycle,
        &fixture.record.project_id,
        &fixture.record.issue_id,
        &crate::product::models::ProviderName::ClaudeCode,
        &fixture.record.id,
    )
    .expect("begin entrypoint plan split run");

    let session = start_work_item_plan_author(
        plan_launch,
        direct_provider,
        input,
        CancellationToken::new(),
        Some(&run_ctx),
    )
    .await
    .expect("logical start via gateway projection");
    drop(session);

    let captured = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("capture adapter input")
        .expect("captured input");
    let observed_cwd = captured.working_directory.clone();
    let observed_target = captured.working_dir.clone();

    assert_eq!(mapped_ref.provider_type, expected_provider);
    assert_eq!(direct_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(observed_cwd, Some(t14_canonical_root(&fixture)));
    assert_eq!(observed_target, member_worktree);
    assert!(
        captured.audit_sink.is_some(),
        "所有 LC 入口必须经 prepare 绑定 run-bound sink(含无通用 tool_policy 角色)"
    );
}

/// B2b1(Task 5.2,REQ-PLN-08,r27 机制重钉):native anchor 收敛 E 聚合根后,
/// 成员锚/无锚的 envelope target 漂移机制结构性消失——任意两次 resolve 的
/// target 三元组恒等(root、无成员身份),显式 resume 的投影比对不再因
/// 锚形态漂移而恒拒;business 成员身份只留在材料面。
#[test]
fn plan_author_launch_anchor_drifts_target_and_fingerprint_without_member_anchor() {
    let fixture = t14_logical_fixture_with_gateway_extras(Arc::new(CapturingStreamingAdapter {
        inputs: mpsc::unbounded_channel().0,
        start_mark: None,
        observed_start_mark: Arc::default(),
    }));
    let canonical_root = t14_canonical_root(&fixture);

    let (first, second) = {
        let engine = fixture.engine.blocking_lock();
        let first = resolve_plan_author_launch(&engine).expect("first launch");
        let second = resolve_plan_author_launch(&engine).expect("second launch");
        (first, second)
    };
    let (PlanAuthorLaunch::Logical(first), PlanAuthorLaunch::Logical(second)) = (&first, &second)
    else {
        panic!("logical session must resolve Logical launches");
    };
    let first_request = first.launch.planning_request().expect("request");
    let second_request = second.launch.planning_request().expect("request");
    for request in [&first_request, &second_request] {
        assert_eq!(request.target.worktree, canonical_root);
        assert!(request.target.checkout_id.is_empty());
        assert!(request.target.logical_repository_id.is_empty());
    }
    assert_eq!(
        first.validated.fingerprint().digest,
        second.validated.fingerprint().digest,
        "同会话两次 resolve 的 envelope 指纹必须恒等——r27 锚漂移恒拒机制已消除"
    );
}

// ---------------------------------------------------------------------------
// T6 fix-loop r2：WIP plan author 家族共享启动口（U4-U7 物理口）trust 分类
// 下沉——七已包 caller 与 followups 四续驱共用 `start_work_item_plan_author`
// 的 Logical 分支，此处对该物理口直测分类合同。
// ---------------------------------------------------------------------------

/// U4-U7 共享物理启动口：Logical 分支内统一分类——Kimi author 条目缺失
/// 时 spawn trust 拒绝以 `provider_trust_missing` 分类＋同源指导＋生成
/// 场景 retry_action 拒收；provider 零启动。
#[tokio::test]
async fn start_work_item_plan_author_logical_classifies_spawn_trust_failure() {
    use super::single_candidate_lc_admission::{
        AliasLcGatewayExtras, LcDialectCapabilitySource, lc_scoped_availability_gate,
    };
    use super::single_candidate_provider_run::ProviderRunFixture;

    let starts = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(CountingStreamingAdapter {
        starts: starts.clone(),
    });
    let trust_home = tempfile::tempdir().expect("isolated trust home");
    let fixture = ProviderRunFixture::new_logical_with_gateway_extras(
        true,
        provider,
        AliasLcGatewayExtras {
            capability: Some(Arc::new(LcDialectCapabilitySource)),
            trust_home: Some(trust_home.path().to_path_buf()),
            availability: Some(lc_scoped_availability_gate(true, true)),
        },
    );
    // B2b1:E native anchor 消费要求 durable 会话记录——直接复用 fixture
    // 的持久 engine(同 gateway/record),author 改 Kimi(workspace trust 面)。
    fixture.engine.lock().await.session.author_provider =
        crate::product::models::ProviderName::KimiCode;
    let plan_launch = {
        let engine = fixture.engine.lock().await;
        resolve_plan_author_launch(&engine).expect("kimi author logical launch")
    };
    let input = streaming_input(fixture.member_checkout_root());

    let result = start_work_item_plan_author(
        plan_launch,
        Arc::new(CountingStreamingAdapter {
            starts: starts.clone(),
        }),
        input,
        CancellationToken::new(),
        None,
    )
    .await;
    let error = match result {
        Ok(_) => panic!("plan author spawn must reject on missing trust"),
        Err(error) => error,
    };

    assert!(
        error.details.contains("provider_trust_missing"),
        "Logical 分支 trust 拒绝必须提升 provider_trust_missing 分类，got: {}",
        error.details
    );
    assert!(
        error
            .details
            .contains("provider trust not established for KimiCode"),
        "原拒绝原文必须保留，got: {}",
        error.details
    );
    assert!(
        error.details.contains("人工登记") && error.details.contains("显式重新生成"),
        "Logical 分支必须携带生成场景 retry_action，got: {}",
        error.details
    );
    assert!(
        error.details.contains(
            &trust_home
                .path()
                .join(".kimi-code")
                .join("workspace-trust")
                .display()
                .to_string()
        ),
        "Logical 分支指导必须同源（真实工件路径），got: {}",
        error.details
    );
    assert_eq!(
        starts.load(Ordering::SeqCst),
        0,
        "trust 拒绝必须发生在 provider 启动之前"
    );
}

/// U4-U7 负例：policy 漂移变体（TOCTOU）即使 trust 同时缺失也不得误附
/// trust 分类——分类下沉后变体判据保持在共享桥内。
#[test]
fn start_work_item_plan_author_drift_variant_stays_bare_at_bridge() {
    use super::single_candidate_lc_admission::{AliasLcGatewayExtras, LcDialectCapabilitySource};
    use super::single_candidate_provider_run::ProviderRunFixture;

    let provider = Arc::new(CountingStreamingAdapter {
        starts: Arc::new(AtomicUsize::new(0)),
    });
    let trust_home = tempfile::tempdir().expect("isolated trust home");
    let fixture = ProviderRunFixture::new_logical_with_gateway_extras(
        true,
        provider,
        AliasLcGatewayExtras {
            capability: Some(Arc::new(LcDialectCapabilitySource)),
            trust_home: Some(trust_home.path().to_path_buf()),
            availability: Some(
                super::single_candidate_lc_admission::lc_scoped_availability_gate(true, true),
            ),
        },
    );
    let gateway = fixture
        .engine
        .blocking_lock()
        .logical_provider_gateway()
        .expect("alias fixture must inject gateway")
        .clone();
    let drift = crate::product::logical_codebase::ProviderGatewayError::PolicyDrift {
        dimension: "config_digest".to_string(),
    };
    let details = gateway.classify_launch_trust_failure_details(
        &drift,
        &crate::product::models::ProviderName::KimiCode,
        &drift.to_string(),
        crate::product::logical_codebase::provider_trust::ProviderTrustScenario::Generation,
    );
    assert_eq!(
        details,
        drift.to_string(),
        "漂移变体不得误附 trust 分类/指导（条目同时缺失也不误标），got: {details}"
    );
}

/// U4-U7 负例：条目已 trusted——即使错误变体是 ProviderUnavailable 也不
/// 得再附 trust 分类（只读判据当前已通过，无可指导事实）。
#[test]
fn start_work_item_plan_author_trusted_entry_stays_bare_at_bridge() {
    use super::single_candidate_lc_admission::{AliasLcGatewayExtras, LcDialectCapabilitySource};
    use super::single_candidate_provider_run::ProviderRunFixture;

    let provider = Arc::new(CountingStreamingAdapter {
        starts: Arc::new(AtomicUsize::new(0)),
    });
    let trust_home = tempfile::tempdir().expect("isolated trust home");
    let fixture = ProviderRunFixture::new_logical_with_gateway_extras(
        true,
        provider,
        AliasLcGatewayExtras {
            capability: Some(Arc::new(LcDialectCapabilitySource)),
            trust_home: Some(trust_home.path().to_path_buf()),
            availability: Some(
                super::single_candidate_lc_admission::lc_scoped_availability_gate(true, true),
            ),
        },
    );
    let gateway = fixture
        .engine
        .blocking_lock()
        .logical_provider_gateway()
        .expect("alias fixture must inject gateway")
        .clone();
    // 人工登记 Kimi 条目（同源 adapter 键算法/记录形状）。
    let adapter =
        crate::product::logical_codebase::provider_trust_adapters::KimiTrustAdapter::for_home(
            trust_home.path(),
        );
    let authority_root = gateway.authority_root().to_path_buf();
    let trust_key =
        crate::product::logical_codebase::provider_trust_adapters::ProviderTrustHomeAdapter::trust_key(
            &adapter, &authority_root,
        );
    let trust_dir = trust_home.path().join(".kimi-code").join("workspace-trust");
    std::fs::create_dir_all(&trust_dir).expect("create kimi trust dir");
    std::fs::write(
        trust_dir.join(&trust_key),
        serde_json::json!({
            "root": authority_root.display().to_string(),
            "trustedAt": 1_u64
        })
        .to_string(),
    )
    .expect("write trusted kimi entry");

    let unavailable = crate::product::logical_codebase::ProviderGatewayError::ProviderUnavailable(
        "provider trust not established for KimiCode: entry absent".to_string(),
    );
    let details = gateway.classify_launch_trust_failure_details(
        &unavailable,
        &crate::product::models::ProviderName::KimiCode,
        &unavailable.to_string(),
        crate::product::logical_codebase::provider_trust::ProviderTrustScenario::Generation,
    );
    assert_eq!(
        details,
        unavailable.to_string(),
        "条目已 trusted 的 ProviderUnavailable 不得再附 trust 分类，got: {details}"
    );
}

// ---------------------------------------------------------------------------
// B2b1（Task 5.2，tasks.md 5.2 / 设计 §3/§4.2/§4.3，anchor_t14）：Plan
// full native 启动与 launch 纪元 map 绑定——E root 锚、prepared map before
// start、native audit 同 run bind、FreshRunning Resolved 收口。
// ---------------------------------------------------------------------------

/// anchor_t14 专用 adapter：start 时快照 durable prepared map（证明
/// prepared-before-start），并立即以固定 native id Completed。durable 定位
/// 经晚绑定 probe（fixture 构造后回填）。
struct T14NativeProbeAdapter {
    inputs: mpsc::UnboundedSender<StreamingProviderInput>,
    observed_prepared:
        Arc<std::sync::Mutex<Vec<(crate::product::models::LogicalLaunchFingerprintState, u64)>>>,
    probe: Arc<std::sync::Mutex<Option<(crate::product::lifecycle_store::LifecycleStore, String)>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for T14NativeProbeAdapter {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        // provider start 时点快照 prepared map——若写入晚于 spawn，此快照
        // 不含本 run 绑定（红判据）。
        if let Some((lifecycle, session_id)) = self.probe.lock().expect("probe").clone() {
            let record = lifecycle
                .get_workspace_session(&session_id)
                .expect("durable session record at provider start");
            *self.observed_prepared.lock().expect("prepared probe") = record
                .logical_launch_fingerprints
                .iter()
                .map(|entry| (entry.state, entry.role_run_seq))
                .collect();
        }
        let _ = self.inputs.send(input);
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let _ = event_tx
                .send(crate::cross_cutting::streaming_provider::ProviderEvent::Completed(
                    crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                        "<ARIA_STRUCTURED_OUTPUT nonce=\"t14\">{\"nonce\":\"t14\",\"work_items\":[]}</ARIA_STRUCTURED_OUTPUT>".to_string(),
                        Some("native-t14-fresh".to_string()),
                    ),
                ))
                .await;
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn start_validated(
        &self,
        validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (input, _launch) = validated.into_parts();
        self.start(input, cancel).await
    }
}

/// B2b1 主用例（tasks.md 5.2 验收全链）：
/// 1) native anchor from E root：envelope target=canonical root、checkout 空；
/// 2) prepared map before start：provider start 时点 durable 已含本 run 的
///    Prepared 绑定（I/O 失败零 start 由 fail-closed 映射保证）；
/// 3) native audit 同 run bind：`persisted_binding.role_run_seq ==
///    actual_start_seq`（handle seq 即 audit sink 绑定 seq）；
/// 4) FreshRunning 的 Resolved 完成收口：fresh_role_run_seq/fresh_native_
///    session_id/fresh_node_id/operation Completed 同 run 对账。
#[tokio::test]
async fn anchor_t14_plan_native_anchor_and_map_are_full_and_durable() {
    use crate::product::models::{
        LcFreshOperationResult, LcFreshOperationState, LcResumeRecoveryRecord,
        LcResumeRecoveryState, LogicalLaunchBasis, LogicalLaunchFingerprintState,
        ProviderConversationRole,
    };

    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let observed_prepared = Arc::new(std::sync::Mutex::new(Vec::new()));
    let probe = Arc::new(std::sync::Mutex::new(None));
    let fixture = t14_logical_fixture_with_gateway_extras(Arc::new(T14NativeProbeAdapter {
        inputs: input_tx,
        observed_prepared: observed_prepared.clone(),
        probe: probe.clone(),
    }));
    let canonical_root = t14_canonical_root(&fixture);
    *probe.lock().expect("bind probe") =
        Some((fixture.lifecycle.clone(), fixture.record.id.clone()));

    // 预置 FreshRunning recovery（dispatch 已受理并 spawn 成功的纪元前态）。
    let expected = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("baseline record");
    let now = chrono::Utc::now().to_rfc3339();
    fixture
        .lifecycle
        .compare_and_save_lc_resume_state(
            &expected,
            LcResumeRecoveryRecord {
                schema_version: 1,
                recovery_id: "recovery_anchor_t14".to_string(),
                failed_provider_node_id: "timeline_node_001".to_string(),
                reason_code: "planning_context_stale".to_string(),
                role: ProviderConversationRole::Author,
                provider: ProviderName::ClaudeCode,
                superseded_native_session_id: Some("native-t14-old".to_string()),
                previous_fingerprint_digest: Some("sha256:t14-old".to_string()),
                previous_session_status: expected.status.clone(),
                previous_stage: "running".to_string(),
                run_context: crate::product::models::LcRecoveryRunContext::LegacyOutlineRebuild {
                    business: crate::product::models::LcRecoveryBusinessContext {
                        entity_id: fixture.record.entity_id.clone(),
                        plan_id: None,
                        workspace_type: crate::product::models::WorkspaceType::WorkItemPlan,
                        business_repository_physical_id: None,
                        business_repository_logical_id: None,
                        business_checkout_id: None,
                        business_repository_path: None,
                        source_story_ids: vec![],
                        source_design_ids: vec![],
                        plan_options_ref: None,
                        artifact_refs: vec![],
                        review_refs: vec![],
                        author_provider: ProviderName::ClaudeCode,
                        effective_reviewer_provider: None,
                        original_node_id: Some("timeline_node_001".to_string()),
                        original_role_run_seq: None,
                    },
                    origin: crate::product::models::LcOutlineRebuildOrigin::LegacyAuthor(
                        crate::product::models::LcLegacyOutlineAuthorPayload {
                            flow:
                                crate::product::work_item_plan_policy::WorkItemPlanFlowKind::Legacy,
                            original_provider_node_id: None,
                            outline_ref: None,
                        },
                    ),
                    previous_snapshot_access_fingerprint: None,
                },
                state: LcResumeRecoveryState::FreshRunning,
                operation_id: Some("op-t14-fresh".to_string()),
                fresh_node_id: None,
                fresh_native_session_id: None,
                fresh_role_run_seq: None,
                fresh_operations: vec![LcFreshOperationResult {
                    operation_id: "op-t14-fresh".to_string(),
                    state: LcFreshOperationState::Running,
                    fresh_node_id: None,
                    reason_code: None,
                    role_run_seq: None,
                    accepted_at: now.clone(),
                    updated_at: now,
                }],
                detected_at: chrono::Utc::now().to_rfc3339(),
                updated_at: chrono::Utc::now().to_rfc3339(),
            },
        )
        .expect("seed FreshRunning recovery");

    // 1) E root 锚（envelope target 三元组）。
    let launch = {
        let engine = fixture.engine.lock().await;
        resolve_plan_author_launch(&engine).expect("resolve logical launch")
    };
    let PlanAuthorLaunch::Logical(plan) = &launch else {
        panic!("logical session must resolve a Logical launch");
    };
    let envelope = plan.validated.envelope();
    assert_eq!(envelope.target.worktree, canonical_root);
    assert!(envelope.target.checkout_id.is_empty());
    assert!(envelope.target.logical_repository_id.is_empty());

    // run 身份 + provider 启动 + 驱动至 Completed + 成功收口（arm 等价链）。
    let run_ctx = super::super::run::begin_plan_split_run(
        &fixture.lifecycle,
        &fixture.record.project_id,
        &fixture.record.issue_id,
        &ProviderName::ClaudeCode,
        &fixture.record.id,
    )
    .expect("begin plan split run");
    let actual_start_seq = run_ctx.handle.role_run_seq;

    let (node_id, full_output) = {
        let mut engine = fixture.engine.lock().await;
        let node_id = engine.begin_work_item_plan_author_run().await;
        let input = streaming_input(fixture.member_checkout_root());
        let session = start_work_item_plan_author(
            launch,
            Arc::new(CountingStreamingAdapter {
                starts: Arc::new(AtomicUsize::new(0)),
            }),
            input,
            CancellationToken::new(),
            Some(&run_ctx),
        )
        .await
        .expect("logical start with run handle");
        let (_command_tx, mut command_rx) = mpsc::channel(8);
        let output = engine
            .drive_work_item_plan_provider_session_to_output(
                Ok(session),
                &mut command_rx,
                node_id.clone(),
                ProviderName::ClaudeCode,
            )
            .await
            .expect("drive to Completed");
        (node_id, output)
    };
    close_plan_split_run(Some(&run_ctx), "t14 prompt", &full_output);

    // 2) prepared map before start（provider start 时点快照含本 run 绑定）。
    let snapshot = observed_prepared.lock().expect("prepared snapshot").clone();
    assert!(
        snapshot.iter().any(|(state, seq)| {
            *state == LogicalLaunchFingerprintState::Prepared && *seq == actual_start_seq
        }),
        "provider start 前 durable 必须已含本 run 的 Prepared 绑定,got: {snapshot:?}"
    );

    let captured = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("capture adapter input")
        .expect("captured input");
    assert!(
        captured.audit_sink.is_some(),
        "run-bound audit sink 必须绑定"
    );

    // 3)+4) durable 终态：map Active/native 绑定 + recovery Resolved 对账。
    let record = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("final record");
    let binding = record
        .logical_launch_fingerprints
        .iter()
        .find(|entry| {
            entry.role == ProviderConversationRole::Author
                && entry.provider == ProviderName::ClaudeCode
                && entry.role_run_seq == actual_start_seq
        })
        .expect("persisted launch binding");
    assert_eq!(binding.state, LogicalLaunchFingerprintState::Active);
    assert_eq!(
        binding.role_run_seq, actual_start_seq,
        "native audit 同 run bind:map 绑定 seq 必须==实际启动 seq"
    );
    assert_eq!(
        binding.native_session_id.as_deref(),
        Some("native-t14-fresh")
    );
    assert_eq!(binding.node_id, node_id);
    assert_eq!(binding.basis, LogicalLaunchBasis::FullLcProjectionV2);
    assert!(
        binding
            .fingerprint_digest
            .contains("lc-session-resume-fingerprint/2")
    );

    let recovery = record.lc_resume_recovery.expect("recovery kept").clone();
    assert_eq!(recovery.state, LcResumeRecoveryState::Resolved);
    assert_eq!(recovery.fresh_role_run_seq, Some(actual_start_seq));
    assert_eq!(
        recovery.fresh_native_session_id.as_deref(),
        Some("native-t14-fresh")
    );
    assert_eq!(recovery.fresh_node_id.as_deref(), Some(node_id.as_str()));
    assert_eq!(
        recovery.fresh_operations[0].state,
        LcFreshOperationState::Completed
    );
    assert_eq!(
        recovery.fresh_operations[0].role_run_seq,
        Some(actual_start_seq)
    );
}
