#[tokio::test]
async fn context_notes_are_included_in_author_prompt_for_all_workspace_types() {
    for (workspace_type, output) in [
        (
            WorkspaceType::Story,
            "# Story Spec\n\n## 功能需求\n- [REQ-001] 记录用户补充上下文。\n\n## 成功标准\n- [AC-001] author prompt 包含补充上下文。\n",
        ),
        (
            WorkspaceType::Design,
            "# Design Spec\n\n## 设计决策\n- [DEC-001] 使用用户补充上下文。\n\n## API 契约\n- 无新增 API。\n",
        ),
        (
            WorkspaceType::WorkItem,
            "# Work Item\n\n## 目标\n- 使用用户补充上下文。\n\n## 验证命令\n- cargo test --locked\n",
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
            output: output.to_string(),
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
        output: "# Story Spec\n\n## 功能需求\n- [REQ-001] 记录旧补充上下文。\n\n## 成功标准\n- [AC-001] author prompt 包含旧补充上下文。\n".to_string(),
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

#[tokio::test]
async fn reviewer_disabled_enters_human_confirm_without_review_node() {
    let (_tmp, store) = setup();
    let (tx, _) = mpsc::channel(64);
    let mut session = make_session("sess_reviewer_disabled");
    session.stage = WorkspaceStage::Running;
    session.reviewer_provider = None;
    session.review_rounds = 0;
    let mut engine = WorkspaceEngine::new(store, tx, session);

    engine.start_review_or_skip().await;

    assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
    assert!(
        !engine
            .timeline_nodes
            .iter()
            .any(|node| node.node_type == TimelineNodeType::ReviewerRun)
    );
}

#[tokio::test]
async fn append_aborted_by_disconnect_creates_node() {
    let (_tmp, store) = setup();
    let (tx, _) = mpsc::channel(64);
    let session = make_session("sess_disconnect_abort");
    let mut engine = WorkspaceEngine::new(store, tx, session);

    let node = engine
        .append_aborted_by_disconnect("run-1".to_string())
        .await
        .unwrap();

    assert_eq!(node.node_type, TimelineNodeType::AbortedByDisconnect);
    assert_eq!(node.status, TimelineNodeStatus::Failed);
    assert!(
        node.summary
            .as_deref()
            .is_some_and(|summary| summary.contains("run-1"))
    );
}

#[tokio::test]
async fn handle_human_confirm_request_change_starts_revision() {
    let (_tmp, store) = setup();
    let (tx, _) = mpsc::channel(64);
    let session = make_session("sess_human_request_change");
    let mut engine = WorkspaceEngine::new(store, tx, session);
    engine.latest_review_verdict = Some(ReviewVerdict {
        verdict: ReviewVerdictType::NeedsHuman,
        comments: "需要人工判断".to_string(),
        summary: "等待人工确认".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::UserConfirmAllowed,
        work_item_plan_review: None,
    });
    engine
        .enter_human_confirm(Some("等待人工确认".to_string()))
        .await;

    let outcome = engine
        .handle_human_confirm(
            HumanConfirmDecision::RequestChange,
            Some(serde_json::json!({"description": "补充边界条件"})),
        )
        .await
        .unwrap();

    assert_eq!(outcome, ReviewDecisionOutcome::StartRevision);
    assert_eq!(engine.session().stage, WorkspaceStage::Revision);
    assert!(engine.timeline_nodes.iter().any(|node| {
        node.node_type == TimelineNodeType::Revision
            && node.status == TimelineNodeStatus::Active
            && node.summary.as_deref() == Some("根据人工反馈返修")
    }));
}

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
            "# Story Spec\n\n## 功能需求\n- [REQ-001] 生成候选草稿。\n\n## 成功标准\n- [AC-001] 候选草稿可进入人工处理。\n",
        ),
        (
            WorkspaceType::Design,
            "# Design Spec\n\n## 设计决策\n- [DEC-001] 生成候选设计。\n\n## 公共组件\n- [CMP-001] 无新增组件。\n",
        ),
        (
            WorkspaceType::WorkItem,
            "# Work Item\n\n## 目标\n- 生成候选实施计划。\n\n## 验证命令\n- cargo test --locked\n",
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
                    output: output.to_string(),
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

#[tokio::test]
async fn author_decision_accept_starts_review_or_final_confirmation() {
    let (_tmp, store) = setup();
    let (tx, _) = mpsc::channel(64);
    let mut session = make_session("sess_author_accept_review");
    session.reviewer_provider = Some(ProviderName::Codex);
    session.review_rounds = 1;
    let mut engine = WorkspaceEngine::new(store, tx, session);
    engine
        .handle_user_message(
            "开始生成".to_string(),
            Arc::new(ImmediateOutputRecordingProvider {
                inputs: Arc::new(Mutex::new(Vec::new())),
                output: "# Story Spec\n\n## 功能需求\n- [REQ-001] 候选。\n\n## 成功标准\n- [AC-001] 可审核。\n".to_string(),
            }),
            empty_provider_commands(),
        )
        .await;

    engine
        .handle_author_decision(AuthorDecision::Accept)
        .await
        .unwrap();

    assert_eq!(engine.session().stage, WorkspaceStage::CrossReview);
    assert!(engine.timeline_nodes.iter().any(|node| {
        node.node_type == TimelineNodeType::ReviewerRun && node.status == TimelineNodeStatus::Active
    }));

    let (_tmp, store) = setup();
    let (tx, _) = mpsc::channel(64);
    let mut session = make_session("sess_author_accept_no_review");
    session.reviewer_provider = None;
    session.review_rounds = 0;
    let mut engine = WorkspaceEngine::new(store, tx, session);
    engine
        .handle_user_message(
            "开始生成".to_string(),
            Arc::new(ImmediateOutputRecordingProvider {
                inputs: Arc::new(Mutex::new(Vec::new())),
                output: "# Story Spec\n\n## 功能需求\n- [REQ-001] 候选。\n\n## 成功标准\n- [AC-001] 可确认。\n".to_string(),
            }),
            empty_provider_commands(),
        )
        .await;

    engine
        .handle_author_decision(AuthorDecision::Accept)
        .await
        .unwrap();

    assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
    assert!(engine.timeline_nodes.iter().any(|node| {
        node.node_type == TimelineNodeType::HumanConfirm
            && node.status == TimelineNodeStatus::Active
    }));
}

#[tokio::test]
async fn author_decision_reject_returns_to_prepare_without_losing_history() {
    let (_tmp, store) = setup();
    let (tx, _) = mpsc::channel(64);
    let mut session = make_session("sess_author_reject");
    session.reviewer_provider = Some(ProviderName::Codex);
    session.review_rounds = 1;
    let mut engine = WorkspaceEngine::new(store, tx, session);
    engine
        .handle_user_message(
            "开始生成".to_string(),
            Arc::new(ImmediateOutputRecordingProvider {
                inputs: Arc::new(Mutex::new(Vec::new())),
                output: "# Story Spec\n\n## 功能需求\n- [REQ-001] 不满意的候选。\n\n## 成功标准\n- [AC-001] 需要重新写。\n".to_string(),
            }),
            empty_provider_commands(),
        )
        .await;

    engine
        .handle_author_decision(AuthorDecision::Reject)
        .await
        .unwrap();

    assert_eq!(engine.session().stage, WorkspaceStage::PrepareContext);
    assert_eq!(engine.session().artifact, None);
    assert!(
        engine
            .session()
            .messages
            .iter()
            .any(|message| message.role == "assistant" && message.content.contains("不满意的候选")),
        "rejected author output should remain in message history"
    );
    assert_eq!(engine.artifact_versions.len(), 1);
    assert!(
        engine.artifact_versions[0]
            .markdown()
            .contains("不满意的候选")
    );
    assert!(
        !engine.artifact_versions[0].is_current,
        "rejected artifact version should remain historical but not active"
    );
    assert!(
        engine.timeline_nodes.iter().any(|node| {
            node.node_type == TimelineNodeType::AuthorConfirm
                && node.status == TimelineNodeStatus::Completed
                && node.summary.as_deref() == Some("用户要求重新编写")
        }),
        "author_confirm node should record the rejection decision"
    );
}

#[tokio::test]
async fn rejected_author_artifact_is_not_restored_after_reconnect() {
    let (tmp, lifecycle_store, mut engine) = persistent_test_engine();
    engine
        .handle_user_message(
            "开始生成".to_string(),
            Arc::new(ImmediateOutputRecordingProvider {
                inputs: Arc::new(Mutex::new(Vec::new())),
                output: "# Story Spec\n\n## 功能需求\n- [REQ-001] 被拒绝候选。\n\n## 成功标准\n- [AC-001] 不应恢复为当前稿。\n".to_string(),
            }),
            empty_provider_commands(),
        )
        .await;

    engine
        .handle_author_decision(AuthorDecision::Reject)
        .await
        .unwrap();

    let session_record = lifecycle_store
        .get_workspace_session(&engine.session().session_id)
        .unwrap();
    let reloaded = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(tmp.path().to_path_buf())),
        lifecycle_store,
        mpsc::channel(64).0,
        WorkspaceSession::from_record(session_record),
    );

    assert_eq!(reloaded.session().stage, WorkspaceStage::PrepareContext);
    assert_eq!(reloaded.session().artifact, None);
    match reloaded.build_session_state() {
        WsOutMessage::SessionState { artifact, .. } => assert_eq!(artifact, None),
        other => panic!("expected SessionState, got {other:?}"),
    }
}

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
            let output = "# Story Spec\n\n\
                ## 功能需求\n\
                - [REQ-001] 生成候选草稿。\n\n\
                ## 成功标准\n\
                - [AC-001] 候选草稿可进入审核。\n";
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: output.to_string(),
                })
                .await;
            let _ = event_tx
                .send(ProviderEvent::Completed {
                    full_output: output.to_string(),
                    provider_session_id: None,
                })
                .await;
        });
        Ok(ProviderSession {
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

#[tokio::test]
async fn handle_user_message_uses_streamed_artifact_when_completed_output_is_summary() {
    let (_tmp, store) = setup();
    let (tx, mut rx) = mpsc::channel(64);
    let session = make_session("sess_streamed_artifact_summary");
    let mut engine = WorkspaceEngine::new(store, tx, session);

    engine
        .handle_user_message(
            "start".to_string(),
            Arc::new(StreamedArtifactSummaryProvider),
            empty_provider_commands(),
        )
        .await;

    engine
        .handle_author_decision(AuthorDecision::Accept)
        .await
        .unwrap();

    assert_eq!(engine.session().stage, WorkspaceStage::CrossReview);
    assert!(engine.session().artifact.as_ref().is_some_and(|artifact| {
        artifact
            .markdown_or_empty()
            .contains("# Streamed Story Spec")
    }));
    assert!(
        drain_engine_events(&mut rx).iter().any(|event| matches!(
            event,
            EngineEvent::ArtifactUpdate { payload, .. }
                if payload.markdown_or_empty().contains("# Streamed Story Spec")
        )),
        "streamed artifact should be published even when Completed.full_output is only a summary"
    );
}

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

struct StreamedArtifactSummaryProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for StreamedArtifactSummaryProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let streamed = "```artifact\n# Streamed Story Spec\n\n\
                ## 功能需求\n\
                - [REQ-001] 使用流式正文中的候选产物。\n\n\
                ## 成功标准\n\
                - [AC-001] Completed 摘要不含 artifact 时仍能进入审核。\n\
                ```";
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: streamed.to_string(),
                })
                .await;
            let _ = event_tx
                .send(ProviderEvent::Completed {
                    full_output: "Story Spec 候选已输出。等待 daemon 处理。".to_string(),
                    provider_session_id: None,
                })
                .await;
        });
        Ok(ProviderSession {
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

#[derive(Default)]
struct DesignArtifactRetryProvider {
    inputs: Arc<Mutex<Vec<StreamingProviderInput>>>,
    calls: Arc<Mutex<u32>>,
}
