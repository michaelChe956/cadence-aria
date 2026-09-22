// F2-B（SC compile 失败教学重驱）：missing_section 类 compile 失败给一次教学重驱
// 自修机会（错误原文进重驱 prompt）；重驱成功→正常继续；重驱再败→终态失败含
// 两轮信息；非 missing_section 错误不触发重驱。

use std::sync::atomic::{AtomicUsize, Ordering};

pub(super) struct SequenceOutputProvider {
    pub(super) outputs: Vec<String>,
    pub(super) inputs: mpsc::UnboundedSender<StreamingProviderInput>,
    pub(super) next: AtomicUsize,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for SequenceOutputProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let _ = self.inputs.send(input);
        let index = self.next.fetch_add(1, Ordering::SeqCst);
        let output = self
            .outputs
            .get(index)
            .or_else(|| self.outputs.last())
            .cloned()
            .unwrap_or_default();
        provider_session_with_output(output).await
    }

    async fn run_streaming(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        unreachable!("sequence provider tests use start")
    }
}

/// 从合法 SC markdown 中删除一个 section 块，制造 missing_section 类 compile 失败。
fn markdown_without_section(story_id: &str, design_id: &str, section_heading: &str) -> String {
    single_candidate_markdown(story_id, design_id)
        .split("\n\n")
        .filter(|chunk| !chunk.starts_with(section_heading))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// 在结构化 section 内追加表外 key，制造 unknown_structured_key 类 compile 失败
/// （所有必需 section/字段仍在，不产生 missing_section）。
fn markdown_with_unknown_structured_key(story_id: &str, design_id: &str) -> String {
    single_candidate_markdown(story_id, design_id).replacen(
        "### Handoff Schema\n- required_fields: commit_sha",
        "### Handoff Schema\n- bogus_key: x\n- required_fields: commit_sha",
        1,
    )
}

pub(super) async fn next_provider_input(
    input_rx: &mut mpsc::UnboundedReceiver<StreamingProviderInput>,
) -> StreamingProviderInput {
    tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("provider input expected")
        .expect("provider input channel open")
}

pub(super) async fn no_more_provider_inputs(
    input_rx: &mut mpsc::UnboundedReceiver<StreamingProviderInput>,
) {
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), input_rx.recv()).await,
            Ok(Some(_))
        ),
        "no further provider invocation is allowed here"
    );
}

pub(super) async fn next_error_message(
    outbound_rx: &mut mpsc::Receiver<OutboundControl>,
) -> String {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let outbound = outbound_rx.recv().await.expect("outbound control expected");
            let OutboundControl::Text(json) = outbound else {
                continue;
            };
            let value: serde_json::Value = serde_json::from_str(&json).expect("outbound json");
            if value["type"] == "error" {
                return value["message"]
                    .as_str()
                    .expect("error message")
                    .to_string();
            }
        }
    })
    .await
    .expect("error outbound expected")
}

#[tokio::test]
async fn single_candidate_compile_missing_section_reredrive_recovers_and_continues() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![
            markdown_without_section(&fixture.story_id, &fixture.design_id, "### Goal"),
            single_candidate_markdown(&fixture.story_id, &fixture.design_id),
        ],
        inputs: input_tx,
        next: AtomicUsize::new(0),
    });
    let (context, _outbound_rx) = single_candidate_context(&fixture, provider);

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
        },
    )
    .await;

    let first_input = next_provider_input(&mut input_rx).await;
    assert!(
        first_input.prompt.contains("[markdown_grammar]"),
        "first round must stay the full markdown author prompt"
    );
    let reredrive_input = next_provider_input(&mut input_rx).await;
    for required in [
        "立即输出完整 work-item-plan markdown source",
        "第一行即文档标题 `# Work Item Plan`",
        "missing_section",
        "Work Item 缺少必需 section",
    ] {
        assert!(
            reredrive_input.prompt.contains(required),
            "teaching re-drive prompt must contain {required}: {}",
            reredrive_input.prompt
        );
    }
    no_more_provider_inputs(&mut input_rx).await;
    wait_for_stage(&fixture.engine, WorkspaceStage::HumanConfirm).await;
    assert_eq!(
        single_candidate_generation_steps_for_session(&fixture.record.id),
        vec!["full_markdown_author", "parse_source_revision", "selector"],
        "re-drive recovery must continue through the canonical compile path exactly once",
    );
    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload single-candidate session");
    assert_eq!(
        durable.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Approval),
    );
    let scope = crate::product::work_item_plan_source_store::SourceStoreScope {
        project_id: durable.project_id.clone(),
        issue_id: durable.issue_id.clone(),
        plan_id: durable.entity_id.clone(),
    };
    let source_store = crate::product::work_item_plan_source_store::WorkItemPlanSourceStore::new(
        fixture.app_paths.clone(),
    );
    let stored = source_store
        .get_source_revision(
            &scope,
            durable
                .work_item_plan_source_revision_ref
                .as_deref()
                .expect("source revision ref"),
        )
        .expect("stored source");
    assert_eq!(
        stored.source,
        single_candidate_markdown(&fixture.story_id, &fixture.design_id),
        "the recovered re-drive output must become the persisted source revision"
    );
}

#[tokio::test]
async fn single_candidate_compile_missing_section_reredrive_failure_is_terminal_with_both_rounds() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![
            markdown_without_section(&fixture.story_id, &fixture.design_id, "### Goal"),
            markdown_without_section(&fixture.story_id, &fixture.design_id, "### Tasks"),
        ],
        inputs: input_tx,
        next: AtomicUsize::new(0),
    });
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider);

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
        },
    )
    .await;

    let _first_input = next_provider_input(&mut input_rx).await;
    let _reredrive_input = next_provider_input(&mut input_rx).await;
    no_more_provider_inputs(&mut input_rx).await;
    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("compile markdown source failed"),
        "{message}"
    );
    assert!(
        message.contains("first round") && message.contains("re-drive round"),
        "terminal failure must carry both rounds' diagnostics: {message}"
    );
    assert!(
        message.matches("missing_section").count() >= 2,
        "terminal failure must contain both rounds' error text: {message}"
    );
    assert!(
        message.contains("请显式重新开始生成"),
        "terminal failure must tell the user how to recover (explicit reopen guidance): {message}"
    );
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Failed,
    )
    .await;
}

#[tokio::test]
async fn single_candidate_compile_unknown_key_failure_stays_terminal_without_reredrive() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![markdown_with_unknown_structured_key(
            &fixture.story_id,
            &fixture.design_id,
        )],
        inputs: input_tx,
        next: AtomicUsize::new(0),
    });
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider);

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
        },
    )
    .await;

    let _first_input = next_provider_input(&mut input_rx).await;
    no_more_provider_inputs(&mut input_rx).await;
    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("compile markdown source failed")
            && message.contains("unknown_structured_key"),
        "non-missing_section compile failure must stay terminal: {message}"
    );
    assert!(
        message.contains("请显式重新开始生成"),
        "single-round terminal failure must also carry the explicit reopen guidance: {message}"
    );
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Failed,
    )
    .await;
}

#[tokio::test]
async fn single_candidate_failed_reopen_claims_next_ledger_key_and_starts_provider() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let language_rules_path = fixture
        .repository_root
        .path()
        .join(".claude/rules/language.md");
    let language_rules = std::fs::read_to_string(&language_rules_path)
        .expect("fixture language rules must exist before the failed attempt");
    std::fs::remove_file(&language_rules_path).expect("remove language rules for first attempt");

    let (first_input_tx, first_input_rx) = mpsc::unbounded_channel();
    let first_provider = Arc::new(RecordingOutputProvider {
        output: single_candidate_markdown(&fixture.story_id, &fixture.design_id),
        inputs: first_input_tx,
    });
    let (first_context, mut first_outbound_rx) = single_candidate_context(&fixture, first_provider);
    handle_workspace_inbound_message(
        first_context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
        },
    )
    .await;
    let _ = next_error_message(&mut first_outbound_rx).await;
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Failed,
    )
    .await;
    drop(first_input_rx);

    std::fs::write(&language_rules_path, language_rules)
        .expect("restore language rules before explicit user reopen");
    let (second_input_tx, mut second_input_rx) = mpsc::unbounded_channel();
    let second_provider = Arc::new(RecordingOutputProvider {
        output: single_candidate_markdown(&fixture.story_id, &fixture.design_id),
        inputs: second_input_tx,
    });
    let (second_context, _second_outbound_rx) = single_candidate_context(&fixture, second_provider);
    handle_workspace_inbound_message(
        second_context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
        },
    )
    .await;

    wait_for_stage(&fixture.engine, WorkspaceStage::HumanConfirm).await;
    let _ = next_provider_input(&mut second_input_rx).await;
    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload reopened session");
    assert_eq!(
        durable
            .provider_start_ledger
            .iter()
            .map(|entry| entry.provider_start_idempotency_key.as_str())
            .collect::<Vec<_>>(),
        vec![
            format!("single_candidate_author:{}:0", fixture.record.id),
            format!("single_candidate_author:{}:1", fixture.record.id),
        ],
        "a failed explicit reopen must claim a new durable provider-start key"
    );
}

#[tokio::test]
async fn single_candidate_completed_reopen_is_rejected_without_starting_or_running() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    {
        let mut engine = fixture.engine.lock().await;
        engine.persist_single_candidate_terminal_phase(
            crate::product::models::SingleCandidatePhase::Completed,
        );
    }

    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(RecordingOutputProvider {
        output: single_candidate_markdown(&fixture.story_id, &fixture.design_id),
        inputs: input_tx,
    });
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider);
    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
        },
    )
    .await;
    let message = next_error_message(&mut outbound_rx).await;
    // F-30：终态守卫先于 store 重臂拦截——错误码 SESSION_ALREADY_CONFIRMED 语义
    // + 会话 id + 重跑出口指引，比旧 store Conflict 文案更可诊断（语义不变：
    // 可见拒绝、不启动 provider、durable 零改写）。
    assert!(
        message.contains("SESSION_ALREADY_CONFIRMED") && message.contains("新建会话"),
        "completed reopen must return a visible terminal rejection: {message}"
    );
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), input_rx.recv()).await,
            Ok(Some(_))
        ),
        "completed reopen must not invoke a provider"
    );
    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload completed session");
    assert_eq!(
        durable.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Completed)
    );
    assert_eq!(
        durable.status,
        crate::product::models::WorkspaceSessionStatus::Confirmed,
        "completed reopen must not pre-burn the durable status to running"
    );
    assert_eq!(
        fixture.engine.lock().await.current_stage(),
        WorkspaceStage::PrepareContext
    );
    assert!(fixture.manager.active_run().await.is_none());
}

#[tokio::test]
async fn single_candidate_reopen_preserves_failed_author_node_terminal_fields() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let node_id = "failed-author-run".to_string();
    let original_completed_at = "2026-09-18T08:09:10Z".to_string();
    let original_summary = "原始 author 失败摘要".to_string();
    {
        let mut engine = fixture.engine.lock().await;
        engine
            .timeline_nodes
            .push(crate::web::workspace_ws_types::TimelineNode {
                node_id: node_id.clone(),
                node_type: crate::web::workspace_ws_types::TimelineNodeType::AuthorRun,
                agent: Some(ProviderName::ClaudeCode),
                stage: crate::web::workspace_ws_types::WorkspaceStage::Running,
                round: None,
                status: crate::web::workspace_ws_types::TimelineNodeStatus::Failed,
                title: "SingleCandidate author".to_string(),
                summary: Some(original_summary.clone()),
                started_at: "2026-09-18T08:00:00Z".to_string(),
                completed_at: Some(original_completed_at.clone()),
                duration_ms: Some(10),
                artifact_ref: None,
                provider_config_snapshot: provider_config(),
                retry: None,
            });
        engine.active_node_id = Some(node_id.clone());
        engine.persist_timeline_nodes();
        engine.persist_single_candidate_terminal_phase(
            crate::product::models::SingleCandidatePhase::Failed,
        );

        engine
            .start_generation(provider_config(), false)
            .await
            .expect("explicit reopen should re-arm a failed SingleCandidate session");

        let node = engine
            .timeline_nodes
            .iter()
            .find(|node| node.node_id == node_id)
            .expect("original failed author node");
        assert_eq!(
            node.status,
            crate::web::workspace_ws_types::TimelineNodeStatus::Failed
        );
        assert_eq!(
            node.completed_at.as_deref(),
            Some(original_completed_at.as_str())
        );
        assert_eq!(node.summary.as_deref(), Some(original_summary.as_str()));
    }
}

/// 永不完成的 provider：一旦被（错误地）启动即可被 starts 计数捕获。
struct HeldStartProvider {
    starts: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for HeldStartProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        let (_event_tx, event_rx) = mpsc::channel(1);
        let (command_tx, _command_rx) = mpsc::channel(1);
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        unreachable!("workspace provider-run tests use start")
    }
}

/// k3 P2 败者让位锚：provider-start 键已被健康持有者领走（在途 run 持有
/// `:{id}:0`，durable phase=Generate）时，迟到的第二条 StartGeneration run 必须
/// 静默让位——不启动 provider、不落 Failed 节点、不广播 Error、不翻转 durable
/// phase。修复前 `Ok(false)` 一律映射 Message：失败节点 + Error 广播 + 会话
/// 呈现与键持有者的健康在途状态矛盾。
#[tokio::test]
async fn single_candidate_late_run_yields_silently_when_start_key_already_claimed() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    // 模拟健康胜者：reserve 已领取首轮键（phase Prepare→Generate，ledger [:0]）。
    {
        let mut engine = fixture.engine.lock().await;
        let claimed = engine
            .reserve_single_candidate_author_start()
            .expect("healthy winner claims the first provider start key");
        assert!(claimed);
    }
    let starts = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(HeldStartProvider {
        starts: starts.clone(),
    });
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider);

    let outbound_errors = Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));
    let recorded = outbound_errors.clone();
    let drain = tokio::spawn(async move {
        while let Some(control) = outbound_rx.recv().await {
            let OutboundControl::Text(text) = control else {
                continue;
            };
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
                && value["type"] == "error"
            {
                recorded
                    .lock()
                    .await
                    .push(value["message"].as_str().unwrap_or("").to_string());
            }
        }
    });

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
        },
    )
    .await;

    // 等待迟到 run 退场（manager 注册被 finish_run 清空）。
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while fixture.manager.active_run().await.is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("late run must exit instead of hanging on the claimed key");
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    assert_eq!(
        starts.load(Ordering::SeqCst),
        0,
        "败者不得启动 provider——键的持有者独占本次启动"
    );
    assert!(
        outbound_errors.lock().await.is_empty(),
        "败者让位不得广播 Error：{:?}",
        outbound_errors.lock().await
    );
    let (failed_nodes, phase) = {
        let engine = fixture.engine.lock().await;
        (
            engine
                .timeline_nodes
                .iter()
                .filter(|node| {
                    node.status == crate::web::workspace_ws_types::TimelineNodeStatus::Failed
                })
                .count(),
            engine.session.single_candidate_phase.clone(),
        )
    };
    assert_eq!(failed_nodes, 0, "败者让位不得产生虚假 Failed 节点");
    assert_eq!(
        phase,
        Some(crate::product::models::SingleCandidatePhase::Generate),
        "败者让位不得翻转 durable phase"
    );
    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload session");
    assert_eq!(
        durable.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Generate),
        "durable phase 必须保持键持有者留下的 Generate"
    );
    drain.abort();
}

/// k3 P2 区分锚的另一半：终态 Failed 且无他者在跑时（恢复/迟到 spawn 形态，
/// 不经过入站 re-arm），无法启动必须保持可见 Message 失败路径——Error 广播 +
/// PrepareContext 回滚是既有恢复入口，不得被让位语义吞掉。
#[tokio::test]
async fn single_candidate_terminal_failed_start_reports_visible_recovery_error() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    {
        let mut engine = fixture.engine.lock().await;
        engine.persist_single_candidate_terminal_phase(
            crate::product::models::SingleCandidatePhase::Failed,
        );
    }
    let starts = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(HeldStartProvider {
        starts: starts.clone(),
    });
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, provider);
    let mut run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        fixture.engine.clone(),
        fixture.workspace_runs.clone(),
        fixture.record.id.clone(),
        fixture.app_paths.clone(),
        fixture.record.clone(),
    );
    run_context.manager = fixture.manager.clone();
    let (outbound_tx, mut outbound_rx) = mpsc::channel(64);

    spawn_provider_run_from_event(
        run_context,
        ProviderRunKind::WorkItemPlanSingleCandidateAuthor,
        None,
        outbound_tx,
    )
    .await
    .expect("spawn the terminal-failed start attempt");

    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("已终态失败"),
        "terminal-failed start must stay visibly rejected, got: {message}"
    );
    assert_eq!(
        starts.load(Ordering::SeqCst),
        0,
        "终态失败会话不得再启动 provider"
    );
    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload session");
    assert_eq!(
        durable.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Failed),
        "durable 终态必须保持 Failed（恢复入口语义）"
    );
    let _ = fixture.manager.abort_active_run().await;
    drop(outbound_rx);
}
