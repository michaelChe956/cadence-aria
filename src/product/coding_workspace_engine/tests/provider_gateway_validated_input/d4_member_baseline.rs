use super::*;

// ================= Task 1.7：D4 生产 seam（LC coding run 全员基线） =================

/// Task 1.7 fixture：三成员 LC——A（active，target 成员）、B（active，非 target
/// 成员）、C（Removed，身份仍保留在 manifest.member_ids）。`removed_keeps_checkout`
/// 控制 C 是否仍留有主 checkout 记录（true=移除后清理前的过渡态，
/// false=清理后的常态）。
struct D4MemberFixture {
    /// target 成员 checkout；当前无读取点，保留 fixture 形状。
    #[allow(dead_code)]
    target_checkout: PathBuf,
    other_active_checkout: PathBuf,
    /// manifest 成员身份（Task 6b 窗口变化用例需要重签 manifest）。
    member_ids: Vec<LogicalRepositoryId>,
}

fn seed_d4_member_codebase(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    root: &Path,
    removed_keeps_checkout: bool,
) -> D4MemberFixture {
    let logical_store = LogicalCodebaseStore::new(store.paths());
    let member_a = LogicalRepositoryId(uuid::Uuid::new_v4());
    let member_b = LogicalRepositoryId(uuid::Uuid::new_v4());
    let removed_member = LogicalRepositoryId(uuid::Uuid::new_v4());
    let repo_a = root.join("repo_a");
    let repo_b = root.join("repo_b");
    let repo_c = root.join("repo_c");
    std::fs::create_dir_all(&repo_a).expect("repo_a dir");
    std::fs::create_dir_all(&repo_b).expect("repo_b dir");
    std::fs::create_dir_all(&repo_c).expect("repo_c dir");
    init_test_git_repo(&repo_a);
    init_test_git_repo(&repo_b);
    init_test_git_repo(&repo_c);

    logical_store
        .save_manifest(
            &attempt.project_id,
            &LogicalCodebaseManifest::new(
                &attempt.project_id,
                store.paths().root().to_path_buf(),
                vec![member_a, member_b, removed_member],
            ),
        )
        .expect("save manifest");

    let now = "2026-10-01T00:00:00Z".to_string();
    for (member_id, status, alias, checkout_path) in [
        (member_a, MemberStatus::Active, "repo_a", &repo_a),
        (member_b, MemberStatus::Active, "repo_b", &repo_b),
        (removed_member, MemberStatus::Removed, "repo_c", &repo_c),
    ] {
        let source_identity = RepositorySourceIdentity::from_git_parts(
            checkout_path,
            checkout_path.join(".git"),
            None,
        );
        let checkout_id = RepositoryCheckoutId(uuid::Uuid::new_v4());
        let keeps_checkout = removed_keeps_checkout || member_id != removed_member;
        logical_store
            .save_member(
                &attempt.project_id,
                &CodebaseMemberRecord {
                    logical_repository_id: member_id,
                    physical_repository_id: "repository_0001".to_string(),
                    alias: alias.to_string(),
                    role: "repository".to_string(),
                    ordinal: 1,
                    source_identity,
                    repo_type: RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: if keeps_checkout {
                        vec![checkout_id]
                    } else {
                        Vec::new()
                    },
                    status,
                    created_at: now.clone(),
                    updated_at: now.clone(),
                },
            )
            .expect("save member");
        if !keeps_checkout {
            continue;
        }
        logical_store
            .save_checkout(
                &attempt.project_id,
                &RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id: member_id,
                    physical_repository_id: "repository_0001".to_string(),
                    kind: CheckoutKind::Main,
                    canonical_path: checkout_path.clone(),
                    checkout_path_hash: "sha256:checkout".to_string(),
                    git_dir_identity: "sha256:git-dir".to_string(),
                    revision: None,
                    availability: CheckoutAvailability::Available,
                    observed_at: now.clone(),
                    created_at: now.clone(),
                    updated_at: now.clone(),
                },
            )
            .expect("save checkout");
    }

    D4MemberFixture {
        target_checkout: repo_a,
        other_active_checkout: repo_b,
        member_ids: vec![member_a, member_b, removed_member],
    }
}

/// D4 seam 探针 adapter：spawn 瞬间读取本次 role run 的 cross-target baseline
/// （存在性 + 内容快照）并计数 spawn；可选在会话期间向 `drift_target` 写入
/// 一次越界文件，模拟 provider 写非 target 成员主 checkout。
struct D4BaselineProbeAdapter {
    baseline_path: PathBuf,
    spawns: Arc<std::sync::atomic::AtomicUsize>,
    baseline_at_spawn: Arc<std::sync::Mutex<Option<serde_json::Value>>>,
    drift_target: Option<PathBuf>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for D4BaselineProbeAdapter {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.spawns
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let baseline = std::fs::read_to_string(&self.baseline_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok());
        *self.baseline_at_spawn.lock().expect("baseline probe mutex") = baseline;
        if let Some(drift_target) = &self.drift_target {
            std::fs::write(drift_target, "out of worktree write\n")
                .expect("simulated non-target member drift");
        }
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(4);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Completed(
                    crate::cross_cutting::streaming_provider::ProviderCompletion::from_output(
                        "d4 probe output".to_string(),
                        None,
                        None,
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
}

/// 以 LC Coder 角色驱动一次真实 provider stream：policy 经
/// `resolve_launch_policy_for_role` resolve，input 经生产工厂
/// `coder_retry_cycle_streaming_input` 构造，spawn 唯一经 gateway
/// （`run_provider_stream_invocation` = coding/retry cycle 的流入口）。
#[allow(clippy::type_complexity)]
async fn drive_logical_coding_run(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    drift_target: Option<PathBuf>,
) -> (
    ProviderInvocationOutcome,
    Arc<std::sync::atomic::AtomicUsize>,
    Arc<std::sync::Mutex<Option<serde_json::Value>>>,
    Arc<GatewayRunAudit>,
) {
    override_coder_to_claude_code(store, attempt);
    let role_run = store
        .create_role_run(
            attempt,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
            CodingRoleRunTrigger::Initial,
            None,
        )
        .expect("create role run");
    drive_logical_coding_run_for_role_run(store, attempt, &role_run, drift_target).await
}

/// [`drive_logical_coding_run`] 的显式 role_run 变体（Task 6b）：调用方先创建
/// role run 并可在其 spawn 前冻结/漂移 D4 基线，再驱动该 role run 的真实流。
#[allow(clippy::type_complexity)]
async fn drive_logical_coding_run_for_role_run(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    role_run: &CodingRoleRun,
    drift_target: Option<PathBuf>,
) -> (
    ProviderInvocationOutcome,
    Arc<std::sync::atomic::AtomicUsize>,
    Arc<std::sync::Mutex<Option<serde_json::Value>>>,
    Arc<GatewayRunAudit>,
) {
    override_coder_to_claude_code(store, attempt);
    let audit = Arc::new(GatewayRunAudit::new());
    let baseline_path = CodingAttemptStore::new(store.paths())
        .attempt_cross_target_baselines_path(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
        )
        .expect("baseline path");
    let probe = Arc::new(D4BaselineProbeAdapter {
        baseline_path,
        spawns: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        baseline_at_spawn: Arc::new(std::sync::Mutex::new(None)),
        drift_target,
    });
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, probe.clone());
    let gateway = build_gateway_with_registry(
        &store.paths(),
        &attempt.project_id,
        Arc::new(registry),
        audit.clone(),
    );
    let (event_tx, _event_rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx)
        .with_logical_provider_gateway(Arc::new(gateway));

    let worktree = attempt.worktree_path.clone().expect("worktree");
    let permission_mode =
        role_permission_mode_for_attempt(store, attempt, CodingProviderRole::Coder)
            .expect("permission mode");
    let (legacy_input, provider_input) = coder_retry_cycle_streaming_input(
        &ProviderName::ClaudeCode,
        "d4 seam probe".to_string(),
        &worktree,
        engine.attempt_provider_stream_log_dir(attempt),
        &attempt.id,
        None,
        permission_mode,
    );
    let policy = engine
        .resolve_launch_policy_for_role(attempt, CodingProviderRole::Coder, &worktree)
        .expect("policy resolves")
        .expect("logical attempt + gateway must produce policy");
    // Task 2.7 re-pin：envelope cwd 已迁 canonical root（lifecycle.rs LC 分支），
    // 生产 cycle（run_coder_with_retry_cycle）在捆绑前把 envelope 冻结 cwd 回填
    // 进 input（spawn 前复验消费 Task 2.5 冻结字段）；本 harness 同步该接线。
    let mut provider_input = provider_input;
    provider_input.working_directory = Some(policy.envelope().working_directory.clone());
    let validated_input = ValidatedStreamingProviderInput::new(provider_input.clone(), policy);
    let (_command_tx, mut command_rx) = mpsc::channel::<CodingRunnerCommand>(1);
    drop(_command_tx);
    let outcome = engine
        .run_provider_stream_invocation(CodingProviderStreamRun {
            attempt,
            node_id: "d4-seam-probe-node",
            role_run: Some(role_run),
            provider: probe.as_ref(),
            legacy_input: &legacy_input,
            input: provider_input,
            provider_name: &ProviderName::ClaudeCode,
            provider_role: CodingProviderRole::Coder,
            command_rx: &mut command_rx,
            allow_legacy_stream_fallback: false,
            timeout: None,
            timeout_reason_code: None,
            suppress_failure_side_effects: false,
            validated_input: Some(validated_input),
        })
        .await;
    (
        outcome,
        probe.spawns.clone(),
        probe.baseline_at_spawn.clone(),
        audit,
    )
}

#[tokio::test]
async fn logical_coding_run_captures_all_member_main_baselines_before_spawn() {
    let (root, store, attempt) = running_attempt_with_worktree();
    // Removed 成员仍保留主 checkout（移除后清理前的过渡态）。
    seed_d4_member_codebase(&store, &attempt, root.path(), true);
    let logical_attempt = with_target_snapshot(&store, &attempt);

    let (outcome, spawns, baseline_at_spawn, audit) =
        drive_logical_coding_run(&store, &logical_attempt, None).await;

    assert!(
        matches!(outcome, ProviderInvocationOutcome::Completed(_)),
        "logical coding run must complete through the gateway seam, got {outcome:?}"
    );
    assert_eq!(
        spawns.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "exactly one provider spawn for the coding run"
    );
    assert_eq!(
        audit.stream_launches(),
        1,
        "the spawn must be counted by the gateway run audit"
    );
    let baseline = baseline_at_spawn
        .lock()
        .expect("baseline probe mutex")
        .clone()
        .expect("cross-target baseline must be persisted before the provider spawns");
    let snapshots = baseline["member_checkouts"]
        .as_array()
        .expect("baseline member_checkouts array");
    assert_eq!(
        snapshots.len(),
        2,
        "baseline window = 全部 active 成员主 checkout（Removed 成员不在 D4 窗口）"
    );
    for snapshot in snapshots {
        let head = snapshot["head_revision"].as_str().unwrap_or_default();
        assert!(
            !head.trim().is_empty(),
            "each active member snapshot must carry a real HEAD revision"
        );
        assert_eq!(
            snapshot["porcelain_status"].as_str(),
            Some(""),
            "clean member checkout must record an empty porcelain status"
        );
    }
}

#[tokio::test]
async fn logical_coding_run_blocks_member_main_checkout_drift() {
    let (root, store, attempt) = running_attempt_with_worktree();
    // Removed 成员的主 checkout 记录已清理（移除后的常态）。
    let members = seed_d4_member_codebase(&store, &attempt, root.path(), false);
    let logical_attempt = with_target_snapshot(&store, &attempt);

    // 会话期间向非 target active 成员主 checkout 写入一次（模拟越界写）。
    let (outcome, spawns, _baseline, _audit) = drive_logical_coding_run(
        &store,
        &logical_attempt,
        Some(members.other_active_checkout.join("trespass.txt")),
    )
    .await;

    assert!(
        matches!(outcome, ProviderInvocationOutcome::Completed(_)),
        "coding run must spawn and complete despite removed-member checkout cleanup, got {outcome:?}"
    );
    assert_eq!(
        spawns.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "drift must happen inside a real spawned run, not a pre-spawn capture failure"
    );

    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);
    let result = engine
        .execute_review_request(&logical_attempt, "origin", "feat: d4 drift")
        .await;
    match result {
        Err(CodingWorkspaceEngineError::CrossTargetDeliveryBlocked(code)) => {
            assert_eq!(code, "cross_target_violation_detected");
        }
        other => panic!("expected CrossTargetDeliveryBlocked, got {other:?}"),
    }
}

// ================= Task 3.2：D4 全角色——CodeReviewer rework cycle =================

/// D4 reviewer-cycle 探针：spawn 时点从 attempt 的 cross-target baselines
/// 目录发现**本次 role run** 刚落盘的基线（capture 先于 spawn，故 spawn 时
/// 目录内恰一份）并读取内容；可选在会话期间向非 target 成员主 checkout
/// 写一次越界文件。会话本体委托 `FakeStreamingProvider`（Reviewer 角色
/// structured output 走真实解析链）。
struct D4ReviewCycleProbeAdapter {
    baselines_root: PathBuf,
    spawns: Arc<std::sync::atomic::AtomicUsize>,
    baseline_at_spawn: Arc<std::sync::Mutex<Option<serde_json::Value>>>,
    drift_target: Option<PathBuf>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for D4ReviewCycleProbeAdapter {
    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.spawns
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let baseline = std::fs::read_dir(&self.baselines_root)
            .ok()
            .and_then(|entries| {
                entries
                    .filter_map(|entry| entry.ok())
                    .find(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            })
            .and_then(|entry| std::fs::read_to_string(entry.path()).ok())
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok());
        *self.baseline_at_spawn.lock().expect("baseline probe mutex") = baseline;
        if let Some(drift_target) = &self.drift_target {
            std::fs::write(drift_target, "reviewer out of worktree write\n")
                .expect("simulated non-target member drift during review run");
        }
        crate::cross_cutting::streaming_provider::FakeStreamingProvider
            .start(input, cancel)
            .await
    }
}

/// Task 3.2（D4 全角色，REQ-ENV-03/REQ-PLN-06）：CodeReviewer rework cycle
/// （生产入口 `execute_code_review` → `run_code_reviewer_with_retry_cycle`）
/// 与 Coder 同享 D4 基线语义：review role run 启动前基线已落盘且窗口=全部
/// active 成员主 checkout；review 会话期间对非 target 成员主 checkout 的
/// 越界写入在交付统一门被检测并阻断（`cross_target_violation_detected`）。
#[tokio::test]
async fn logical_coding_review_baseline_detects_cross_target_mutation() {
    let (root, store, attempt) = running_attempt_with_worktree();
    init_test_git_repo(attempt.worktree_path.as_ref().expect("worktree"));
    // Removed 成员主 checkout 已清理（移除后常态）：窗口恰为 2 个 active 成员。
    let members = seed_d4_member_codebase(&store, &attempt, root.path(), false);
    let logical_attempt = with_target_snapshot(&store, &attempt);

    let baselines_root = CodingAttemptStore::new(store.paths())
        .attempt_cross_target_baselines_root(
            &logical_attempt.project_id,
            &logical_attempt.issue_id,
            &logical_attempt.id,
        );
    let probe = Arc::new(D4ReviewCycleProbeAdapter {
        baselines_root,
        spawns: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        baseline_at_spawn: Arc::new(std::sync::Mutex::new(None)),
        drift_target: Some(members.other_active_checkout.join("trespass.txt")),
    });
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, probe.clone());
    let audit = Arc::new(GatewayRunAudit::new());
    let gateway = build_gateway_with_registry(
        &store.paths(),
        &logical_attempt.project_id,
        Arc::new(registry),
        audit.clone(),
    );
    let (event_tx, _event_rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx)
        .with_logical_provider_gateway(Arc::new(gateway));

    let report = engine
        .execute_code_review(&logical_attempt, probe.as_ref())
        .await
        .expect("reviewer cycle must run through the gateway seam");
    assert_eq!(
        report.verdict,
        crate::product::coding_models::ReviewVerdict::Approve,
        "review cycle must complete (drift detection is post-hoc, not in-run)"
    );
    assert_eq!(
        probe.spawns.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "exactly one reviewer spawn for the rework cycle"
    );
    assert_eq!(
        audit.stream_launches(),
        1,
        "the reviewer spawn must be audited by the gateway"
    );

    // D4 全角色：review role run 的基线同样先于 spawn 落盘，窗口=2 个
    // active 成员（Removed 成员不在窗口）。
    let baseline = probe
        .baseline_at_spawn
        .lock()
        .expect("baseline probe mutex")
        .clone()
        .expect("cross-target baseline must be persisted before the reviewer spawns");
    let snapshots = baseline["member_checkouts"]
        .as_array()
        .expect("baseline member_checkouts array");
    assert_eq!(
        snapshots.len(),
        2,
        "review-role baseline window = 全部 active 成员主 checkout"
    );
    for snapshot in snapshots {
        assert!(
            !snapshot["head_revision"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .is_empty(),
            "each active member snapshot must carry a real HEAD revision"
        );
    }
    assert!(
        members.other_active_checkout.join("trespass.txt").exists(),
        "probe 漂移写必须真实发生在 review run 内"
    );

    // 交付统一门：review run 内的越界写阻断交付。
    let (delivery_tx, _delivery_rx) = mpsc::channel(8);
    let delivery_engine =
        CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), delivery_tx);
    match delivery_engine
        .execute_review_request(&logical_attempt, "origin", "feat: d4 reviewer drift")
        .await
    {
        Err(CodingWorkspaceEngineError::CrossTargetDeliveryBlocked(code)) => {
            assert_eq!(code, "cross_target_violation_detected");
        }
        other => panic!("expected CrossTargetDeliveryBlocked, got {other:?}"),
    }
}

// ================= Task 2.6：LC Coder/retry root cwd 重绑（REQ-ENV-10/ENV-11） =================

/// spawn 探针：记录每次 spawn 时点的 effective cwd 并计数；会话立即完成
/// （与 D4 探针同构，聚焦 cwd/计数两个观测维度）。
struct RootCwdProbeAdapter {
    spawns: Arc<std::sync::atomic::AtomicUsize>,
    cwd_at_spawns: Arc<std::sync::Mutex<Vec<PathBuf>>>,
}

impl RootCwdProbeAdapter {
    fn new() -> Self {
        Self {
            spawns: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            cwd_at_spawns: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for RootCwdProbeAdapter {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.spawns
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.cwd_at_spawns
            .lock()
            .expect("root cwd probe mutex")
            .push(input.effective_working_directory().to_path_buf());
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(4);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Completed(
                    crate::cross_cutting::streaming_provider::ProviderCompletion::from_output(
                        "root cwd probe output".to_string(),
                        None,
                        None,
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
}

/// 以 LC Coder 角色驱动一次真实 coder retry cycle（生产入口
/// `run_coder_with_retry_cycle`）：policy 在 cycle 内 resolve，input 经生产
/// 工厂 `coder_retry_cycle_streaming_input` 构造，spawn 唯一经 gateway。
/// `gateway` 传 `None` 复现「LC attempt 未注入 gateway」的 fail-closed 面。
async fn drive_coder_retry_cycle(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    gateway: Option<Arc<LogicalCodebaseProviderGateway>>,
    probe: &RootCwdProbeAdapter,
) -> Result<ProviderRetryCycleSuccess, CodingWorkspaceEngineError> {
    override_coder_to_claude_code(store, attempt);
    let (event_tx, _event_rx) = mpsc::channel(16);
    let mut engine =
        CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);
    if let Some(gateway) = gateway {
        engine = engine.with_logical_provider_gateway(gateway);
    }
    // 生产链在进入 coder cycle 前把 stage 推进到 Coding（coding.rs 同款）。
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::Coding,
        )
        .expect("stage to Coding");
    let node = engine
        .create_coding_timeline_node(&attempt)
        .expect("timeline node");
    let role_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
            CodingRoleRunTrigger::Initial,
            Some(node.id.clone()),
        )
        .expect("role run");
    let worktree = attempt.worktree_path.clone().expect("worktree");
    let (_command_tx, mut command_rx) = mpsc::channel::<CodingRunnerCommand>(1);
    drop(_command_tx);
    engine
        .run_coder_with_retry_cycle(CoderRetryCycleInput {
            attempt: &attempt,
            initial_node: node,
            initial_role_run: role_run,
            provider: probe,
            provider_name: &ProviderName::ClaudeCode,
            worktree_path: &worktree,
            initial_prompt: "root cwd probe".to_string(),
            fresh_prompt: "root cwd probe fresh".to_string(),
            initial_prompt_mode: CodingPromptMode::FullConversation,
            initial_resume_provider_session_id: None,
            command_rx: &mut command_rx,
        })
        .await
}

/// Task 2.6（REQ-ENV-10/ENV-11）：LC Coder/retry 的 cwd 重绑 canonical root
/// ——spawn 时点 provider 进程 cwd == gateway 冻结的 authority root（manifest
/// `provider_context_root`），恰一次 spawn 经 gateway；writable root 不随 cwd
/// 扩大（恒=attempt target worktree，REQ-ENV-03）。
#[tokio::test]
async fn logical_coder_rebinds_root_cwd_without_expanding_writable_root() {
    let (root, store, attempt) = running_attempt_with_worktree();
    let _root = root;
    init_test_git_repo(attempt.worktree_path.as_ref().unwrap());
    seed_logical_codebase_checkout(&store, &attempt);
    let logical_attempt = with_target_snapshot(&store, &attempt);
    override_coder_to_claude_code(&store, &logical_attempt);

    let probe = Arc::new(RootCwdProbeAdapter::new());
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, probe.clone());
    let gateway = build_gateway_with_registry(
        &store.paths(),
        &logical_attempt.project_id,
        Arc::new(registry),
        Arc::new(GatewayRunAudit::new()),
    );
    let canonical_root = gateway.authority_root().to_path_buf();
    let gateway = Arc::new(gateway);

    // writable root 不随 cwd 扩大：root launch 解析出的 envelope 保持
    // cwd=canonical root、唯一 writable root=target worktree（target 来自
    // attempt snapshot）。
    let (policy_tx, _policy_rx) = mpsc::channel(32);
    let policy_engine =
        CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), policy_tx)
            .with_logical_provider_gateway(gateway.clone());
    let worktree = logical_attempt.worktree_path.clone().expect("worktree");
    let policy = policy_engine
        .resolve_coder_root_launch_policy(&logical_attempt, &worktree)
        .expect("root policy resolve returns Ok")
        .expect("logical attempt + gateway must produce root policy");
    assert_eq!(policy.envelope().working_directory, canonical_root);
    assert_eq!(policy.envelope().writable_roots, vec![worktree]);

    let success = drive_coder_retry_cycle(&store, &logical_attempt, Some(gateway), probe.as_ref())
        .await
        .expect("logical coder retry cycle must complete through the gateway");

    assert_eq!(
        success.outcome.full_output, "root cwd probe output",
        "probe output must round-trip through the coder retry cycle"
    );
    assert_eq!(
        probe.spawns.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "exactly one provider spawn for the lc coder run"
    );
    let cwd_at_spawns = probe
        .cwd_at_spawns
        .lock()
        .expect("root cwd probe mutex")
        .clone();
    assert_eq!(
        cwd_at_spawns,
        vec![canonical_root.clone()],
        "provider process cwd must be rebound to the canonical lc root"
    );
}

/// Task 2.6：LC attempt 未注入 gateway（无 validated launch）时 coder retry
/// cycle fail-closed——零 spawn，错误携带 `logical_provider_gateway_required`
/// 稳定码，绝不回落 legacy 直连（Task 12 门在 cycle 内保持关闭）。
#[tokio::test]
async fn logical_coder_without_validated_gateway_zero_spawns() {
    let (root, store, attempt) = running_attempt_with_worktree();
    let _root = root;
    let logical_attempt = with_target_snapshot(&store, &attempt);

    let probe = Arc::new(RootCwdProbeAdapter::new());
    let error = match drive_coder_retry_cycle(&store, &logical_attempt, None, probe.as_ref()).await
    {
        Err(error) => error,
        Ok(_) => panic!("logical coder cycle must fail closed without an injected gateway"),
    };

    assert!(
        error
            .to_string()
            .contains("logical_provider_gateway_required"),
        "expected logical_provider_gateway_required error, got: {error:?}"
    );
    assert_eq!(
        probe.spawns.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "zero provider spawns without a validated gateway launch"
    );
}

// ================= Task 6b：D4 baseline freshness 只读复检 seam（REQ-LCG-03/07） =================

use crate::product::coding_attempt_store::StableCode;
use crate::product::coding_workspace_engine::cross_target_check::{
    capture_cross_target_baseline, revalidate_cross_target_baseline,
};

fn new_coder_role_run(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> CodingRoleRun {
    store
        .create_role_run(
            attempt,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
            CodingRoleRunTrigger::Initial,
            None,
        )
        .expect("create role run")
}

/// 冻结本 role run 的 D4 基线（模拟准入/验证时点的首采），返回 baseline 文件
/// 路径。Task 6b 用它在 spawn 前制造「冻结早于 spawn」的复检窗口。
fn freeze_d4_baseline_for_role_run(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    role_run: &CodingRoleRun,
) -> PathBuf {
    capture_cross_target_baseline(&store.paths(), attempt, &role_run.id)
        .expect("admission-time baseline freeze");
    CodingAttemptStore::new(store.paths())
        .attempt_cross_target_baselines_path(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
        )
        .expect("baseline path")
}

/// Task 6b（REQ-LCG-03/07 D4 基线 freshness）：基线冻结与 spawn 之间发生漂移
/// （非 target active 成员主 checkout 被写）时，spawn 前复检 seam 必须
/// fail-closed 阻断本次 role run——provider 子进程计数为 0，冻结基线保持
/// 原样（不得静默重冻结把漂移洗白成新基线）。
#[tokio::test]
async fn lcg_t06_d4_baseline_drift_before_spawn_zero_child() {
    let (root, store, attempt) = running_attempt_with_worktree();
    // Removed 成员主 checkout 已清理（移除后常态）：窗口恰为 2 个 active 成员。
    let members = seed_d4_member_codebase(&store, &attempt, root.path(), false);
    let logical_attempt = with_target_snapshot(&store, &attempt);

    // 冻结时点（准入/验证）先采集本 role run 的基线，再在 spawn 前制造漂移。
    let role_run = new_coder_role_run(&store, &logical_attempt);
    let baseline_path = freeze_d4_baseline_for_role_run(&store, &logical_attempt, &role_run);
    std::fs::write(
        members.other_active_checkout.join("trespass.txt"),
        "drift between freeze and spawn\n",
    )
    .expect("pre-spawn drift into non-target member main checkout");

    let (outcome, spawns, _baseline_at_spawn, audit) =
        drive_logical_coding_run_for_role_run(&store, &logical_attempt, &role_run, None).await;

    let spawn_count_after_d4_drift = spawns.load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(spawn_count_after_d4_drift, 0);
    assert!(
        !matches!(outcome, ProviderInvocationOutcome::Completed(_)),
        "drifted baseline must fail the spawn pre-gate, got {outcome:?}"
    );
    let outcome_text = format!("{outcome:?}");
    assert!(
        outcome_text.contains("cross_target_baseline_capture_failed")
            && outcome_text.contains("cross_target_violation_detected"),
        "expected cross_target drift to fail the capture seam, got: {outcome_text}"
    );
    assert_eq!(
        audit.stream_launches(),
        0,
        "gateway must not launch any stream when the frozen baseline has drifted"
    );

    // 冻结基线不得被漂移后的重采洗白：文件仍是首采的干净快照。
    let frozen: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&baseline_path).expect("frozen baseline survives"),
    )
    .expect("frozen baseline json");
    for snapshot in frozen["member_checkouts"]
        .as_array()
        .expect("frozen member_checkouts array")
    {
        assert_eq!(
            snapshot["porcelain_status"].as_str(),
            Some(""),
            "frozen baseline must keep the admission-time clean snapshot"
        );
    }
}

/// Task 6b 正例：基线冻结后无漂移时，同 role run 重入的 spawn 前复检放行——
/// 恰一次 spawn 经 gateway，会话正常完成（复检不产生假阳性阻断）。
#[tokio::test]
async fn lcg_t06_d4_baseline_fresh_reentry_spawns_once() {
    let (root, store, attempt) = running_attempt_with_worktree();
    let _members = seed_d4_member_codebase(&store, &attempt, root.path(), false);
    let logical_attempt = with_target_snapshot(&store, &attempt);

    let role_run = new_coder_role_run(&store, &logical_attempt);
    freeze_d4_baseline_for_role_run(&store, &logical_attempt, &role_run);

    let (outcome, spawns, baseline_at_spawn, audit) =
        drive_logical_coding_run_for_role_run(&store, &logical_attempt, &role_run, None).await;

    assert!(
        matches!(outcome, ProviderInvocationOutcome::Completed(_)),
        "fresh baseline must not block the spawn, got {outcome:?}"
    );
    assert_eq!(
        spawns.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "exactly one provider spawn when the frozen baseline is still fresh"
    );
    assert_eq!(
        audit.stream_launches(),
        1,
        "the spawn must be audited by the gateway run audit"
    );
    assert!(
        baseline_at_spawn
            .lock()
            .expect("baseline probe mutex")
            .is_some(),
        "the frozen baseline must still be readable at spawn"
    );
}

/// Task 6b 负例（seam 直连）：冻结后任一 active 成员主 checkout 漂移 →
/// `revalidate_cross_target_baseline` 只读复检返回 `cross_target_violation_detected`。
#[test]
fn lcg_t06_revalidate_blocks_drifted_member_checkout() {
    let (root, store, attempt) = running_attempt_with_worktree();
    let members = seed_d4_member_codebase(&store, &attempt, root.path(), false);
    let logical_attempt = with_target_snapshot(&store, &attempt);
    let role_run = new_coder_role_run(&store, &logical_attempt);
    freeze_d4_baseline_for_role_run(&store, &logical_attempt, &role_run);

    std::fs::write(
        members.other_active_checkout.join("trespass.txt"),
        "out of worktree write\n",
    )
    .expect("post-freeze drift");

    assert_eq!(
        revalidate_cross_target_baseline(&store.paths(), &logical_attempt, &role_run.id),
        Err(StableCode::CrossTargetViolationDetected)
    );
}

/// Task 6b 负例（seam 直连）：冻结基线文件缺失（崩溃重启/丢失）→ 复检返回
/// `cross_target_baseline_missing`——证据不足 fail-closed，绝不折算成放行。
#[test]
fn lcg_t06_revalidate_blocks_missing_baseline_file() {
    let (root, store, attempt) = running_attempt_with_worktree();
    seed_d4_member_codebase(&store, &attempt, root.path(), false);
    let logical_attempt = with_target_snapshot(&store, &attempt);
    let role_run = new_coder_role_run(&store, &logical_attempt);
    let baseline_path = freeze_d4_baseline_for_role_run(&store, &logical_attempt, &role_run);

    std::fs::remove_file(&baseline_path).expect("simulate baseline loss");

    assert_eq!(
        revalidate_cross_target_baseline(&store.paths(), &logical_attempt, &role_run.id),
        Err(StableCode::CrossTargetBaselineMissing)
    );
}

/// Task 6b 负例（重采全部 active main checkout）：冻结后 LC 新增 active 成员
/// （窗口增长）→ 复检把「现窗口 ≠ 冻结窗口」判为差异阻断；交付统一门同语义
/// 阻断（旧 frozen-window detect 检不出窗口增长，新 seam 必须拦下）。
#[tokio::test]
async fn lcg_t06_revalidate_resamples_full_active_main_window() {
    let (root, store, attempt) = running_attempt_with_worktree();
    let members = seed_d4_member_codebase(&store, &attempt, root.path(), false);
    let logical_attempt = with_target_snapshot(&store, &attempt);
    let role_run = new_coder_role_run(&store, &logical_attempt);
    freeze_d4_baseline_for_role_run(&store, &logical_attempt, &role_run);

    // 冻结窗口 {A, B} → 现窗口 {A, B, D}：新增干净 active 成员 D。
    let repo_d = root.path().join("repo_d");
    std::fs::create_dir_all(&repo_d).expect("repo_d dir");
    init_test_git_repo(&repo_d);
    let member_d = LogicalRepositoryId(uuid::Uuid::new_v4());
    let logical_store = LogicalCodebaseStore::new(store.paths());
    let mut member_ids = members.member_ids.clone();
    member_ids.push(member_d);
    logical_store
        .save_manifest(
            &logical_attempt.project_id,
            &LogicalCodebaseManifest::new(
                &logical_attempt.project_id,
                store.paths().root().to_path_buf(),
                member_ids,
            ),
        )
        .expect("re-sign manifest with the new active member");
    let now = "2026-10-01T00:00:00Z".to_string();
    let source_identity =
        RepositorySourceIdentity::from_git_parts(&repo_d, repo_d.join(".git"), None);
    let checkout_d = RepositoryCheckoutId(uuid::Uuid::new_v4());
    logical_store
        .save_member(
            &logical_attempt.project_id,
            &CodebaseMemberRecord {
                logical_repository_id: member_d,
                physical_repository_id: "repository_0001".to_string(),
                alias: "repo_d".to_string(),
                role: "repository".to_string(),
                ordinal: 2,
                source_identity,
                repo_type: RepositoryType::Unknown,
                tech_stack: Vec::new(),
                owner: None,
                tags: Vec::new(),
                default_ref: None,
                checkout_ids: vec![checkout_d],
                status: MemberStatus::Active,
                created_at: now.clone(),
                updated_at: now.clone(),
            },
        )
        .expect("save member d");
    logical_store
        .save_checkout(
            &logical_attempt.project_id,
            &RepositoryCheckoutRecord {
                checkout_id: checkout_d,
                logical_repository_id: member_d,
                physical_repository_id: "repository_0001".to_string(),
                kind: CheckoutKind::Main,
                canonical_path: repo_d,
                checkout_path_hash: "sha256:checkout".to_string(),
                git_dir_identity: "sha256:git-dir".to_string(),
                revision: None,
                availability: CheckoutAvailability::Available,
                observed_at: now.clone(),
                created_at: now.clone(),
                updated_at: now,
            },
        )
        .expect("save checkout d");

    // seam 直连：重采全部 active main checkout，窗口增长即差异。
    assert_eq!(
        revalidate_cross_target_baseline(&store.paths(), &logical_attempt, &role_run.id),
        Err(StableCode::CrossTargetViolationDetected)
    );

    // 交付统一门同语义：窗口变化必须阻断交付。
    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), event_tx);
    match engine
        .execute_review_request(&logical_attempt, "origin", "feat: window grew")
        .await
    {
        Err(CodingWorkspaceEngineError::CrossTargetDeliveryBlocked(code)) => {
            assert_eq!(code, "cross_target_violation_detected");
        }
        other => panic!("expected CrossTargetDeliveryBlocked, got {other:?}"),
    }
}

/// Task 6b 负例（fail-closed 证据不足）：冻结后成员主 checkout 的 git 元数据
/// 不可采样（如 `.git` 丢失）→ 复检返回 `cross_target_store_failure`——证据
/// 不足只能阻断，不得折算成「无漂移」或 Unknown 放行（复检不是用户 normal
/// action 的旁路）。
#[test]
fn lcg_t06_revalidate_fails_closed_on_unsampled_checkout() {
    let (root, store, attempt) = running_attempt_with_worktree();
    let members = seed_d4_member_codebase(&store, &attempt, root.path(), false);
    let logical_attempt = with_target_snapshot(&store, &attempt);
    let role_run = new_coder_role_run(&store, &logical_attempt);
    freeze_d4_baseline_for_role_run(&store, &logical_attempt, &role_run);

    std::fs::remove_dir_all(members.other_active_checkout.join(".git"))
        .expect("destroy member git metadata");

    assert_eq!(
        revalidate_cross_target_baseline(&store.paths(), &logical_attempt, &role_run.id),
        Err(StableCode::CrossTargetStoreFailure)
    );
}
