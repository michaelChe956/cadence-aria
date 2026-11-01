#[tokio::test]
async fn context_notes_are_included_in_author_prompt_for_all_workspace_types() {
    for (workspace_type, output) in [
        (
            WorkspaceType::Story,
            complete_story_artifact("记录用户补充上下文。", "author prompt 包含补充上下文。"),
        ),
        (
            WorkspaceType::Design,
            complete_design_artifact("使用用户补充上下文。", "无新增 API。"),
        ),
        (
            WorkspaceType::WorkItem,
            complete_work_item_artifact("使用用户补充上下文。"),
        ),
    ] {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let mut session = make_session("sess_context_note_prompt");
        session.workspace_type = workspace_type.clone();
        session.reviewer_provider = None;
        let mut engine = WorkspaceEngine::new(store, tx, session);
        let inputs = Arc::new(Mutex::new(Vec::new()));
        let provider = Arc::new(ImmediateOutputRecordingProvider {
            inputs: inputs.clone(),
            output,
        });

        engine
            .append_context_note("用户补充：必须覆盖 n=10 -> 89。".to_string())
            .await
            .unwrap();
        engine
            .handle_user_message("开始生成".to_string(), provider, empty_provider_commands())
            .await;

        let inputs = inputs.lock().unwrap();
        let prompt = &inputs
            .first()
            .expect("author provider should receive input")
            .prompt;
        assert!(
            prompt.contains("用户补充：必须覆盖 n=10 -> 89。"),
            "{workspace_type:?} author prompt should include prepare context note, got: {prompt}"
        );
        assert!(
            prompt.contains("开始生成"),
            "{workspace_type:?} author prompt should include generation request, got: {prompt}"
        );
    }
}

#[tokio::test]
async fn legacy_context_note_timeline_nodes_are_included_in_author_prompt() {
    let (_tmp, store) = setup();
    let (tx, _) = mpsc::channel(64);
    let mut session = make_session("sess_legacy_context_note_prompt");
    session.reviewer_provider = None;
    let mut engine = WorkspaceEngine::new(store, tx, session);
    let inputs = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(ImmediateOutputRecordingProvider {
        inputs: inputs.clone(),
        output: complete_story_artifact("记录旧补充上下文。", "author prompt 包含旧补充上下文。"),
    });

    engine
        .append_completed_timeline_event(
            TimelineNodeType::ContextNote,
            WorkspaceStage::PrepareContext,
            "上下文补充".to_string(),
            Some("旧现场补充：Story Spec 必须使用 n=10 -> 89。".to_string()),
            TimelineNodeStatus::Completed,
            false,
        )
        .await;
    engine
        .handle_user_message("开始生成".to_string(), provider, empty_provider_commands())
        .await;

    let inputs = inputs.lock().unwrap();
    let prompt = &inputs
        .first()
        .expect("author provider should receive input")
        .prompt;
    assert!(
        prompt.contains("旧现场补充：Story Spec 必须使用 n=10 -> 89。"),
        "author prompt should include legacy timeline-only context note, got: {prompt}"
    );
}

#[tokio::test]
async fn start_generation_locks_provider_and_creates_node() {
    let (_tmp, store) = setup();
    let (tx, _) = mpsc::channel(64);
    let session = make_session("sess_start_generation");
    let mut engine = WorkspaceEngine::new(store, tx, session);
    let snapshot = ProviderConfigSnapshot {
        author: ProviderName::Codex,
        reviewer: Some(ProviderName::ClaudeCode),
        review_rounds: 1,
        permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
    };

    let (node, locked) = engine
        .start_generation(snapshot.clone(), true)
        .await
        .unwrap();

    assert_eq!(node.node_type, TimelineNodeType::StartGeneration);
    assert_eq!(node.status, TimelineNodeStatus::Completed);
    assert_eq!(engine.session().stage, WorkspaceStage::Running);
    assert_eq!(engine.session().author_provider, ProviderName::Codex);
    assert_eq!(
        engine.session().reviewer_provider,
        Some(ProviderName::ClaudeCode)
    );
    assert_eq!(engine.session().review_rounds, 1);
    match locked {
        WsOutMessage::ProviderLocked {
            snapshot: locked_snapshot,
            locked_at,
        } => {
            assert_eq!(locked_snapshot, snapshot);
            assert!(!locked_at.is_empty());
        }
        _ => panic!("expected ProviderLocked"),
    }
}

// F-30（v34 复验，标本 issue_0003/session_0008）：已确认终态会话上重跑
// 「开始生成」此前走未定义路径——完整轮生成后挂零决策控件门、durable 停留
// 旧状态、timeline 零新节点。入口必须明确拒绝：错误码 SESSION_ALREADY_CONFIRMED
// 语义 + 可诊断消息（含 session id 与重跑出口指引），且不建节点、不推进状态机。
#[tokio::test]
async fn start_generation_rejects_confirmed_session_without_side_effects() {
    let (_tmp, store) = setup();
    let (tx, _rx) = mpsc::channel(64);
    let mut session = make_session("sess_start_generation_confirmed");
    // from_record 口径（mappings.rs）：Confirmed 投影为 stage Completed。
    session.session_status = crate::product::models::WorkspaceSessionStatus::Confirmed;
    session.stage = WorkspaceStage::Completed;
    let mut engine = WorkspaceEngine::new(store, tx, session);
    let snapshot = ProviderConfigSnapshot {
        author: ProviderName::Codex,
        reviewer: None,
        review_rounds: 0,
        permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
    };

    let err = engine
        .start_generation(snapshot, false)
        .await
        .expect_err("confirmed terminal session must reject start_generation");

    assert!(
        err.contains("SESSION_ALREADY_CONFIRMED"),
        "error should carry the terminal-state code, got: {err}"
    );
    assert!(
        err.contains("sess_start_generation_confirmed"),
        "error should name the session for diagnosis, got: {err}"
    );
    assert!(
        err.contains("修订") && err.contains("新建会话"),
        "error should honestly redirect reruns to revision/new session, got: {err}"
    );
    // 不建节点、不架门、不推进状态机、不留活动 run。
    // 不建节点、不架门、不推进状态机、不留活动 run：initial_timeline 的
    // 「准备上下文」节点原样保留（Active），无 StartGeneration 节点混入。
    assert_eq!(engine.current_stage(), WorkspaceStage::Completed);
    assert!(
        engine
            .timeline_nodes
            .iter()
            .all(|node| node.node_type != TimelineNodeType::StartGeneration),
        "no StartGeneration node may be appended for a terminal session"
    );
    assert!(
        engine
            .timeline_nodes
            .iter()
            .any(|node| node.status == TimelineNodeStatus::Active),
        "the pre-existing prepare_context node must stay untouched (no gate, no closure)"
    );
    assert!(engine.active_run_id().is_none());
}

// F-30 同族：Terminated 终态同口径拒绝（错误码区分，重跑出口只有新会话）。
#[tokio::test]
async fn start_generation_rejects_terminated_session() {
    let (_tmp, store) = setup();
    let (tx, _rx) = mpsc::channel(64);
    let mut session = make_session("sess_start_generation_terminated");
    session.session_status = crate::product::models::WorkspaceSessionStatus::Terminated;
    session.stage = WorkspaceStage::Completed;
    let mut engine = WorkspaceEngine::new(store, tx, session);
    let snapshot = ProviderConfigSnapshot {
        author: ProviderName::Codex,
        reviewer: None,
        review_rounds: 0,
        permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
    };

    let err = engine
        .start_generation(snapshot, false)
        .await
        .expect_err("terminated terminal session must reject start_generation");

    assert!(
        err.contains("SESSION_TERMINATED"),
        "error should carry the terminal-state code, got: {err}"
    );
    assert!(
        err.contains("sess_start_generation_terminated"),
        "error should name the session for diagnosis, got: {err}"
    );
    assert_eq!(engine.current_stage(), WorkspaceStage::Completed);
    assert!(
        engine
            .timeline_nodes
            .iter()
            .all(|node| node.node_type != TimelineNodeType::StartGeneration),
        "no StartGeneration node may be appended for a terminal session"
    );
}

// F-30 守卫边界：Failed 不是拦截对象——看门狗/编译失败后的显式重跑是既有
// 语义（finish_failed_run 回 Open；SingleCandidate Failed 走 store 重臂，
// 见 lifecycle_store workspace_single_candidate 显式重开用例）。
#[tokio::test]
async fn start_generation_still_allows_failed_session_rerun() {
    let (_tmp, store) = setup();
    let (tx, _rx) = mpsc::channel(64);
    let mut session = make_session("sess_start_generation_failed");
    session.session_status = crate::product::models::WorkspaceSessionStatus::Failed;
    session.stage = WorkspaceStage::PrepareContext;
    let mut engine = WorkspaceEngine::new(store, tx, session);
    let snapshot = ProviderConfigSnapshot {
        author: ProviderName::Codex,
        reviewer: None,
        review_rounds: 0,
        permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
    };

    let (node, _) = engine
        .start_generation(snapshot, false)
        .await
        .expect("failed session stays rerunnable");

    assert_eq!(node.node_type, TimelineNodeType::StartGeneration);
    assert_eq!(engine.current_stage(), WorkspaceStage::Running);
}

// 退役留档（T5/REQ-RET-02）：`reviewer_disabled_legacy_accept_enters_human_confirm_without_review_node` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

#[tokio::test]
async fn append_aborted_by_disconnect_creates_node() {
    let (_tmp, store) = setup();
    let (tx, _) = mpsc::channel(64);
    let session = make_session("sess_disconnect_abort");
    let mut engine = WorkspaceEngine::new(store, tx, session);

    let node = engine
        .append_aborted_by_disconnect("run-1".to_string(), "connection-1".to_string())
        .await
        .unwrap();

    assert_eq!(node.node_type, TimelineNodeType::AbortedByDisconnect);
    assert_eq!(node.status, TimelineNodeStatus::Failed);
    assert!(
        node.summary
            .as_deref()
            .is_some_and(|summary| summary.contains("run-1"))
    );
    assert!(
        node.summary
            .as_deref()
            .is_some_and(|summary| summary.contains("connection-1"))
    );
}

// 退役留档（T5/REQ-RET-02）：`handle_human_confirm_request_change_starts_revision` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`human_confirm_request_change_requires_context_after_untrusted_review` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`human_confirm_request_change_requires_context_for_untrusted_review_across_workspace_routes` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

#[tokio::test]
async fn set_provider_updates_author_and_reviewer() {
    let (_tmp, store) = setup();
    let (tx, _) = mpsc::channel(64);
    let session = make_session("sess_005");
    let mut engine = WorkspaceEngine::new(store, tx, session);

    assert_eq!(engine.session().author_provider, ProviderName::ClaudeCode);
    assert_eq!(
        engine.session().reviewer_provider,
        Some(ProviderName::Codex)
    );

    engine.set_provider("author", ProviderName::Codex).unwrap();
    assert_eq!(engine.session().author_provider, ProviderName::Codex);

    engine
        .set_provider("reviewer", ProviderName::ClaudeCode)
        .unwrap();
    assert_eq!(
        engine.session().reviewer_provider,
        Some(ProviderName::ClaudeCode)
    );

    let err = engine.set_provider("unknown", ProviderName::Fake);
    assert!(err.is_err());
}

#[tokio::test]
async fn author_completion_enters_author_confirm_for_all_workspace_types() {
    for (workspace_type, output) in [
        (
            WorkspaceType::Story,
            complete_story_artifact("生成候选草稿。", "候选草稿可进入人工处理。"),
        ),
        (
            WorkspaceType::Design,
            complete_design_artifact("生成候选设计。", "无新增 API。"),
        ),
        (
            WorkspaceType::WorkItem,
            complete_work_item_artifact("生成候选单个可执行任务。"),
        ),
    ] {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let mut session = make_session("sess_author_confirm");
        session.workspace_type = workspace_type.clone();
        session.reviewer_provider = Some(ProviderName::Codex);
        session.review_rounds = 1;
        let mut engine = WorkspaceEngine::new(store, tx, session);

        engine
            .handle_user_message(
                "开始生成".to_string(),
                Arc::new(ImmediateOutputRecordingProvider {
                    inputs: Arc::new(Mutex::new(Vec::new())),
                    output,
                }),
                empty_provider_commands(),
            )
            .await;

        assert_eq!(
            engine.session().stage,
            WorkspaceStage::AuthorConfirm,
            "{workspace_type:?} should pause after author output"
        );
        assert!(
            engine
                .timeline_nodes
                .iter()
                .any(|node| node.node_type == TimelineNodeType::AuthorConfirm
                    && node.status == TimelineNodeStatus::Active),
            "{workspace_type:?} should create an active author_confirm node"
        );
        assert!(
            !engine
                .timeline_nodes
                .iter()
                .any(|node| node.node_type == TimelineNodeType::ReviewerRun),
            "{workspace_type:?} should not start reviewer before user accepts author output"
        );
        assert!(
            engine.session().artifact.is_some(),
            "{workspace_type:?} author output should remain visible while waiting for decision"
        );
    }
}

// 退役留档（T5/REQ-RET-02）：`author_decision_accept_starts_review_or_final_confirmation` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`author_decision_reject_returns_guidance_error_without_reset` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`rejected_author_artifact_survives_reject_error_after_reconnect` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

struct RecordingStreamingProvider {
    provider_type: Arc<Mutex<Option<ProviderType>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for RecordingStreamingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        *self.provider_type.lock().unwrap() = Some(input.provider_type.clone());
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let output = complete_story_artifact("生成候选草稿。", "候选草稿可进入审核。");
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: output.clone(),
                })
                .await;
            let _ = event_tx
                .send(ProviderEvent::Completed(
                    crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                        output, None,
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

#[tokio::test]
async fn handle_user_message_uses_author_provider_and_publishes_artifact_for_confirmation() {
    let (_tmp, store) = setup();
    let (tx, mut rx) = mpsc::channel(64);
    let mut session = make_session("sess_006");
    session.author_provider = ProviderName::Codex;
    let mut engine = WorkspaceEngine::new(store, tx, session);
    let provider_type = Arc::new(Mutex::new(None));
    let provider = RecordingStreamingProvider {
        provider_type: provider_type.clone(),
    };

    engine
        .handle_user_message(
            "start".to_string(),
            Arc::new(provider),
            empty_provider_commands(),
        )
        .await;

    assert_eq!(*provider_type.lock().unwrap(), Some(ProviderType::Codex));
    assert_eq!(engine.session().stage, WorkspaceStage::AuthorConfirm);
    assert!(engine.session().artifact.as_ref().is_some_and(|artifact| {
        let artifact = artifact.markdown_or_empty();
        artifact.contains("## 功能需求") && artifact.contains("## 成功标准")
    }));

    let mut saw_artifact = false;
    let mut saw_author_confirm = false;
    while let Ok(event) = rx.try_recv() {
        match event {
            EngineEvent::ArtifactUpdate { payload, .. }
                if payload.markdown_or_empty().contains("## 功能需求")
                    && payload.markdown_or_empty().contains("## 成功标准") =>
            {
                saw_artifact = true;
            }
            EngineEvent::StageChange { stage } if stage == "author_confirm" => {
                saw_author_confirm = true;
            }
            _ => {}
        }
    }
    assert!(
        saw_artifact,
        "provider completion should update the artifact pane"
    );
    assert!(
        saw_author_confirm,
        "provider completion should wait for author confirmation"
    );
}

// 退役留档（T5/REQ-RET-02）：`handle_user_message_uses_streamed_artifact_when_completed_output_is_summary` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

#[tokio::test]
async fn handle_user_message_retries_once_when_design_author_completes_without_artifact() {
    let (_tmp, store) = setup();
    let (tx, mut rx) = mpsc::channel(64);
    let mut session = make_session("sess_design_artifact_retry");
    session.workspace_type = WorkspaceType::Design;
    session.entity_id = "design_spec_0001".to_string();
    session.reviewer_provider = None;
    session.review_rounds = 0;
    let mut engine = WorkspaceEngine::new(store, tx, session);
    let provider = Arc::new(DesignArtifactRetryProvider::default());

    engine
        .handle_user_message(
            "start".to_string(),
            provider.clone(),
            empty_provider_commands(),
        )
        .await;

    assert_eq!(*provider.calls.lock().unwrap(), 2);
    let inputs = provider.inputs.lock().unwrap();
    assert_eq!(inputs.len(), 2);
    assert!(
        inputs[1].prompt.contains("上一轮已结束")
            && inputs[1].prompt.contains("没有输出完整 artifact")
            && inputs[1]
                .prompt
                .contains("立即输出完整 ```artifact``` Design Spec"),
        "retry prompt should force a complete Design Spec artifact, got: {}",
        inputs[1].prompt
    );
    assert_eq!(
        inputs[1].resume_provider_session_id.as_deref(),
        Some("design-retry-session-1")
    );
    drop(inputs);

    assert_eq!(engine.session().stage, WorkspaceStage::AuthorConfirm);
    assert!(engine.session().artifact.as_ref().is_some_and(|artifact| {
        let artifact = artifact.markdown_or_empty();
        artifact.contains("## 设计决策") && artifact.contains("## 公共组件")
    }));
    assert!(
        drain_engine_events(&mut rx).iter().any(|event| matches!(
            event,
            EngineEvent::ArtifactUpdate { payload, .. }
                if payload.markdown_or_empty().contains("# Retried Design Spec")
        )),
        "retry artifact should be published"
    );
}

#[derive(Default)]
struct DesignArtifactRetryProvider {
    inputs: Arc<Mutex<Vec<StreamingProviderInput>>>,
    calls: Arc<Mutex<u32>>,
}
