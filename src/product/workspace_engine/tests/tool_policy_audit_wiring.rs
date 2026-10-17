// F3 修复轮 P1-2（REQ-ENV-09/D7）：workspace reviewer/repair/revision 三路的
// 策略会话 durable 审计 sink 接线测试。与 author 主流（part_31）同法：持久
// engine（lifecycle_store 存在）下 probe 记录每次 start 的 policy/sink 携带。
// 本文件经 include! 进入 tests 模块，直接共享 part 文件的作用域与 helpers。

/// 记录每次 start 是否携带 tool_policy/audit_sink 的队列输出探针。
struct PolicySinkQueuedProvider {
    outputs: Arc<Mutex<VecDeque<String>>>,
    starts: Arc<AtomicUsize>,
    sink_seen: Arc<Mutex<Vec<bool>>>,
    policy_seen: Arc<Mutex<Vec<bool>>>,
}

impl PolicySinkQueuedProvider {
    fn new(outputs: Vec<String>) -> Self {
        Self {
            outputs: Arc::new(Mutex::new(outputs.into())),
            starts: Arc::new(AtomicUsize::new(0)),
            sink_seen: Arc::new(Mutex::new(Vec::new())),
            policy_seen: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for PolicySinkQueuedProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        self.policy_seen
            .lock()
            .unwrap()
            .push(input.tool_policy.is_some());
        self.sink_seen
            .lock()
            .unwrap()
            .push(input.audit_sink.is_some());
        let template = self
            .outputs
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| "# Story Spec\n\n## 范围\n修订后的版本。".to_string());
        let output = input
            .structured_output_contract
            .as_ref()
            .map(|contract| template.replace("__NONCE__", &contract.nonce))
            .unwrap_or(template);
        let completion = ProviderCompletion::from_output(
            output,
            input.structured_output_contract.as_ref(),
            Some("probe-session-1".to_string()),
        );
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(4);
        event_tx
            .send(ProviderEvent::Completed(completion))
            .await
            .unwrap();
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by WorkspaceEngine",
            0,
        ))
    }
}

/// 持久 engine fixture（lifecycle_store 存在，Story 流，reviewer=Codex 可 repair）。
async fn persistent_policy_engine(session_id: &str) -> (TempDir, WorkspaceEngine) {
    let root = tempfile::tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    let record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "p1".to_string(),
            issue_id: "i1".to_string(),
            entity_id: "story_spec_0001".to_string(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: Some(ProviderName::Codex),

            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("create session");
    let checkpoint_store = Arc::new(CheckpointStore::new(paths.issue_lifecycle_root("p1", "i1")));
    let (tx, _rx) = mpsc::channel(64);
    let mut engine = WorkspaceEngine::new_persistent(
        checkpoint_store,
        lifecycle,
        tx,
        WorkspaceSession::from_record(record),
    );
    engine.session.session_id = session_id.to_string();
    engine.session.artifact = Some(artifact_payload("# Story Spec\n\n需要审核的候选版本"));
    (root, engine)
}

#[tokio::test]
async fn workspace_review_and_repair_policy_runs_receive_durable_audit_sink() {
    let (_tmp, mut engine) = persistent_policy_engine("sess_review_repair_policy_sink").await;
    engine.start_review().await;

    let revise_json = r#"{
        "verdict": "revise",
        "summary": "需要返修",
        "findings": [{
            "severity": "must_fix",
            "message": "缺少失败路径",
            "evidence": "Artifact 未覆盖登录失败",
            "required_action": "补充失败路径"
        }]
    }"#;
    let probe = Arc::new(PolicySinkQueuedProvider::new(vec![
        missing_json_nonce_output(revise_json),
        valid_structured_output(revise_json),
    ]));
    engine
        .drive_review_session(probe.clone(), empty_provider_commands())
        .await;

    // 批2 Task 11（T20 前中间态）：review/repair 输入不再携带 deny——纪律=
    // ReviewerVerify+窗口合同（真窗归 T20）；run-bound durable sink 按角色保留。
    assert_eq!(probe.starts.load(Ordering::SeqCst), 2);
    assert_eq!(
        *probe.policy_seen.lock().unwrap(),
        vec![false, false],
        "review/repair inputs must not carry deny policy (T20 intermediate)"
    );
    assert_eq!(
        *probe.sink_seen.lock().unwrap(),
        vec![true, true],
        "review/repair policy inputs must carry the run-bound durable audit sink"
    );
}

// 退役留档（T5/REQ-RET-02）：`workspace_revision_policy_run_receives_durable_audit_sink` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

/// 批2 Task 11（T20/T13 前中间态）：reviewer run 为非策略会话——改用无
/// 三联动前置的 completed-only wire fixture（thread/start 无策略前置），
/// 驱动干净收口；engine 侧审计分配断言不受影响。
fn workspace_policy_audit_fixture() -> std::path::PathBuf {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/provider/codex_app_server_completed_only_fixture.sh");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&path)
            .unwrap_or_else(|error| panic!("fixture metadata {}: {error}", path.display()))
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions)
            .unwrap_or_else(|error| panic!("chmod fixture: {error}"));
    }
    path
}

// ---- F3 Task 4.1 修复轮（REQ-ENV-09/GC10）：workspace 侧审计通道严格分离回归 ----

/// I2（真实事件链证明）：策略 reviewer run 用真实 CodexProvider + wire fixture
/// 经 engine `drive_review_session` 走真实 start 路径——provider 在握手成功后、
/// `start` 返回前写 `provider_start`，审批即时决策落 `approval_decision`，全部
/// 经 engine 注入的 run-bound sink 落到 `tool-policy-run-audit/` 分区；分区外
/// 的既有 lifecycle 产物不含任何 tool-policy 事件标记。（手动 append 属
/// LifecycleStore 单元测试用途，由 lifecycle_store/tests/tool_policy_audit.rs
/// 覆盖，本 wiring 测试不再伪造事件。）
/// 批2 Task 13（合同驱动 durable 分区，B2T11 裁定缺口补齐）：reviewer wire
/// 面去 deny 后，adapter 侧 durable provider_start 写由 run-bound audit 合同
/// 驱动（sink 在位即写首行，不再以 deny intent 为门）；本测试经真实 start
/// 路径锁定 run 事件文件首行 provider_start 与通道分离。ReviewerVerify 真窗
/// 接线与审批 durable 面归 T20。
#[tokio::test]
async fn workspace_policy_review_run_keeps_engine_audit_allocation_via_real_provider_chain() {
    use crate::cross_cutting::codex_provider::CodexProvider;
    use crate::cross_cutting::streaming_provider::ProviderVersionSupplier;

    let (root, mut engine) = persistent_policy_engine("sess_audit_real_chain").await;
    engine.start_review().await;

    let supplier: ProviderVersionSupplier =
        std::sync::Arc::new(|| Ok("codex 0.124.0-ws-audit-fixture".to_string()));
    let provider = Arc::new(
        CodexProvider::new(workspace_policy_audit_fixture()).with_version_supplier(supplier),
    );
    engine
        .drive_review_session(provider, empty_provider_commands())
        .await;

    let aria_root = root.path().join(".aria");
    let partition_dir = aria_root
        .join("tool-policy-run-audit")
        .join("sess_audit_real_chain");
    fn collect_files_recursive(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_files_recursive(&path, out);
            } else {
                out.push(path);
            }
        }
    }
    let mut partition_files = Vec::new();
    collect_files_recursive(&partition_dir, &mut partition_files);
    // `role-run-seq.jsonl` 是 seq 分配高水位 marker，不是事件文件，排除。
    let run_files: Vec<_> = partition_files
        .iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name != "role-run-seq.jsonl")
                .unwrap_or(false)
        })
        .cloned()
        .collect();
    // 批2 Task 13：合同驱动——去 deny reviewer run 的 durable run 事件文件
    // 首行为 provider_start（role=reviewer、digest 空串、danger 基线）；
    // engine 侧审计分配在位（高水位 marker）。
    assert!(
        !run_files.is_empty(),
        "contract-bound reviewer run must write durable partition run files: {run_files:?}"
    );
    for run_file in &run_files {
        let content = std::fs::read_to_string(run_file).unwrap();
        let first_line = content.lines().next().unwrap();
        let first: serde_json::Value = serde_json::from_str(first_line).unwrap();
        assert_eq!(
            first["event_type"], "provider_start",
            "reviewer run file must start with provider_start: {first_line}"
        );
        assert_eq!(first["role"], "reviewer");
        assert_eq!(first["sandbox"], "danger-full-access");
        assert_eq!(
            first["tool_policy_canonical_digest"], "",
            "non-policy contract run carries no tool-policy canonical digest"
        );
    }
    assert!(
        partition_dir.join("role-run-seq.jsonl").exists(),
        "engine must still allocate the reviewer tool-policy role run seq (audit kept post-deny)"
    );

    // 分区之外的既有 lifecycle 产物不含任何 tool-policy 事件标记（通道分离）。
    let mut all_files = Vec::new();
    collect_files_recursive(&aria_root, &mut all_files);
    assert!(!all_files.is_empty());
    for file in &all_files {
        if file.starts_with(&partition_dir) {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(file) {
            for marker in [
                "\"tool_policy_canonical_digest\"",
                "\"event_type\":\"approval_decision\"",
                "\"event_type\":\"protocol_warning\"",
                "\"event_type\":\"session_terminated\"",
            ] {
                assert!(
                    !content.contains(marker),
                    "non-policy lifecycle artifacts must not carry tool-policy events: {} has {marker}",
                    file.display()
                );
            }
        }
    }
}

/// I3（非策略侧收窄为 sink wiring only）：workspace 引擎没有 Executor/Coder
/// 真实 drive 入口——聚合初始化 provider turns 属 logical_codebase 层（经 gateway
/// 启动），因此这里无法做非策略 run 的 execution audit 正向断言；该正向断言
/// 由 coding 侧 `coding_coder_and_policy_runs_keep_audit_channels_strictly_
/// separated`（真实 Coder invocation + role-run-events 产物扫描）覆盖。本测试
/// 只锁定 workspace 侧的非策略 input 永不接收 tool-policy sink（幂等跳过）。
#[tokio::test]
async fn workspace_non_policy_input_never_receives_tool_policy_sink_wiring_only() {
    let (root, engine) = persistent_policy_engine("sess_audit_non_policy_wiring").await;

    let executor_input = crate::cross_cutting::streaming_provider::StreamingProviderInput {
        working_directory: None,
        baseline_tree: None,
        tool_policy: None,
        audit_sink: None,
        provider_type: crate::protocol::contracts::ProviderType::Codex,
        role: crate::protocol::contracts::AdapterRole::Executor,
        prompt: "executor non-policy".to_string(),
        working_dir: std::env::temp_dir(),
        workspace_session_id: Some("sess_audit_non_policy_wiring".to_string()),
        resume_provider_session_id: None,
        permission_mode: crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
        structured_output_contract: None,
        env_vars: Default::default(),
        timeout_secs: 30,
    };
    let untouched = engine.attach_tool_policy_audit(executor_input);
    assert_eq!(untouched.tool_policy, None);
    assert!(
        untouched.audit_sink.is_none(),
        "non-policy executor input must never receive a tool-policy audit sink"
    );
    // 该会话从未跑过策略 run：分区目录不得因非策略 input 的接线尝试而产生。
    assert!(
        !root.path().join(".aria/tool-policy-run-audit").exists(),
        "non-policy input must not create the tool-policy-run-audit partition"
    );
}
