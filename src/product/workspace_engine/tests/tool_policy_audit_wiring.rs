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
