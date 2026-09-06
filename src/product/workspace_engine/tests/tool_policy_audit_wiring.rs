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
    /// Task 4.1：捕获 engine 实际注入的 run-bound sink（审计通道分离回归用）。
    captured_sinks: Arc<
        Mutex<Vec<Option<Arc<dyn crate::cross_cutting::tool_policy_audit::ToolPolicyAuditSink>>>>,
    >,
}

impl PolicySinkQueuedProvider {
    fn new(outputs: Vec<String>) -> Self {
        Self {
            outputs: Arc::new(Mutex::new(outputs.into())),
            starts: Arc::new(AtomicUsize::new(0)),
            sink_seen: Arc::new(Mutex::new(Vec::new())),
            policy_seen: Arc::new(Mutex::new(Vec::new())),
            captured_sinks: Arc::new(Mutex::new(Vec::new())),
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
        self.captured_sinks
            .lock()
            .unwrap()
            .push(input.audit_sink.clone());
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
            reviewer_provider: ProviderName::Codex,
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
    let (_tmp, mut engine) =
        persistent_policy_engine("sess_review_repair_policy_sink").await;
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

    // reviewer 首轮与 repair 轮（P1-2 的两路）都必须携带 policy + run-bound sink。
    assert_eq!(probe.starts.load(Ordering::SeqCst), 2);
    assert_eq!(
        *probe.policy_seen.lock().unwrap(),
        vec![true, true],
        "review/repair inputs must carry the deny policy"
    );
    assert_eq!(
        *probe.sink_seen.lock().unwrap(),
        vec![true, true],
        "review/repair policy inputs must carry the run-bound durable audit sink"
    );
}

#[tokio::test]
async fn workspace_revision_policy_run_receives_durable_audit_sink() {
    let (_tmp, mut engine) = persistent_policy_engine("sess_revision_policy_sink").await;
    engine.session.stage = WorkspaceStage::AuthorConfirm;

    engine
        .handle_author_decision(AuthorDecision::Revise {
            feedback: "补充失败路径".to_string(),
        })
        .await
        .expect("author feedback should enter revision");

    let probe = Arc::new(PolicySinkQueuedProvider::new(Vec::new()));
    engine
        .drive_revision_session(probe.clone(), empty_provider_commands())
        .await;

    // artifact 提取失败会触发既有 artifact retry（第二次 start）；断言每次
    // revision start 都携带 policy + sink。
    assert!(
        probe.starts.load(Ordering::SeqCst) >= 1,
        "revision drive must start the provider"
    );
    assert!(
        probe.policy_seen.lock().unwrap().iter().all(|seen| *seen),
        "revision inputs must carry the deny policy"
    );
    assert!(
        probe.sink_seen.lock().unwrap().iter().all(|seen| *seen),
        "revision policy inputs must carry the run-bound durable audit sink"
    );
}

// ---- F3 Task 4.1（REQ-ENV-09/GC10）：workspace 侧审计通道严格分离回归 ----

/// 策略角色的 canonical 事件只落 `tool-policy-run-audit/` 分区；非策略
/// （Executor/Coder 档）input 幂等跳过 sink 接线，不产生任何 durable 策略
/// 记录；分区之外的既有 lifecycle/execution 产物不含 tool-policy 事件。
#[tokio::test]
async fn workspace_policy_and_non_policy_audit_channels_stay_separated() {
    use crate::cross_cutting::tool_policy_audit::{
        DurableToolPolicyEvent, ProviderStartAudit,
    };

    let (root, mut engine) = persistent_policy_engine("sess_audit_isolation").await;
    engine.start_review().await;

    let pass_json = r#"{
        "verdict": "pass",
        "summary": "通过",
        "findings": []
    }"#;
    let probe = Arc::new(PolicySinkQueuedProvider::new(vec![
        missing_json_nonce_output(pass_json),
        valid_structured_output(pass_json),
    ]));
    engine
        .drive_review_session(probe.clone(), empty_provider_commands())
        .await;
    assert!(
        probe.starts.load(Ordering::SeqCst) >= 1,
        "review drive must start the policy provider"
    );
    let sinks = probe.captured_sinks.lock().unwrap().clone();
    assert!(
        sinks.iter().all(|sink| sink.is_some()),
        "policy review starts must carry the engine-attached durable sink"
    );

    // 经 engine 注入的 run-bound sink 写 canonical 事件（adapter 语义：首行
    // provider_start，随后其余三类），验证只落 durable 分区。
    let sink = sinks
        .into_iter()
        .find_map(|sink| sink)
        .expect("captured durable sink");
    sink.append_bound(DurableToolPolicyEvent::ProviderStart(ProviderStartAudit {
        provider: "codex".to_string(),
        role: "reviewer".to_string(),
        workspace_session_id: "sess_audit_isolation".to_string(),
        provider_session_id: "thread-isolation-1".to_string(),
        tool_policy_canonical_digest: "digest-isolation".to_string(),
        argv: Vec::new(),
        sandbox: Some("read-only".to_string()),
        approval_policy: Some("on-request".to_string()),
        provider_version: "codex 0.153.4".to_string(),
        adapter_dialect: "codex-app-server-rpc".to_string(),
    }))
    .expect("provider_start append via engine sink");
    sink.append_bound(DurableToolPolicyEvent::SessionTerminated(
        crate::cross_cutting::tool_policy_audit::SessionTerminatedAudit {
            reason_code: "completed".to_string(),
        },
    ))
    .expect("session_terminated append via engine sink");

    let aria_root = root.path().join(".aria");
    let partition_dir = aria_root.join("tool-policy-run-audit");
    let partition_file =
        partition_dir.join("sess_audit_isolation").join("0.jsonl");
    let content = std::fs::read_to_string(&partition_file)
        .unwrap_or_else(|error| panic!("partition file {}: {error}", partition_file.display()));
    let lines: Vec<serde_json::Value> = content
        .lines()
        .map(|line| serde_json::from_str(line).expect("partition jsonl line"))
        .collect();
    assert_eq!(lines[0]["event_type"], "provider_start");
    assert_eq!(
        lines[0]["workspace_session_id"], "sess_audit_isolation",
        "LifecycleStore must stamp the file-key workspace id"
    );
    assert_eq!(lines[1]["event_type"], "session_terminated");
    for line in &lines {
        assert!(
            matches!(
                line["event_type"].as_str(),
                Some(
                    "provider_start"
                        | "approval_decision"
                        | "protocol_warning"
                        | "session_terminated"
                )
            ),
            "tool-policy partition must carry only canonical events: {line}"
        );
    }

    // 非策略（Executor 档）input：幂等跳过 sink 接线，不产生任何策略通道记录。
    let executor_input = crate::cross_cutting::streaming_provider::StreamingProviderInput {
        tool_policy: None,
        audit_sink: None,
        provider_type: crate::protocol::contracts::ProviderType::Codex,
        role: crate::protocol::contracts::AdapterRole::Executor,
        prompt: "executor non-policy".to_string(),
        working_dir: std::env::temp_dir(),
        workspace_session_id: Some("sess_audit_isolation".to_string()),
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

    // 全 .aria 树（排除 tool-policy 分区自身）不含任何 tool-policy 事件标记。
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
