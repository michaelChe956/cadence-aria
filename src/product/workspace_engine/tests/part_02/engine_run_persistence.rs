// part_02 拆分（large_file_guard 1200 行上限）：持久引擎 helper 与流式/许可/结论落盘用例族经 include! 挂载，模块作用域与用例名不变，行为零变化。

fn persistent_test_engine() -> (TempDir, LifecycleStore, WorkspaceEngine) {
    let (tmp, checkpoint_store) = setup();
    let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let (tx, _) = mpsc::channel(64);
    let session_record = lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput { project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        entity_id: "story_spec_0001".to_string(),
        workspace_type: WorkspaceType::Story,
        author_provider: ProviderName::ClaudeCode,
        reviewer_provider: Some(ProviderName::Codex),

        review_rounds: 2,
        superpowers_enabled: true, openspec_enabled: true, work_item_plan_options: None, })
        .unwrap();
    let session = WorkspaceSession::from_record(session_record);
    let engine =
        WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store.clone(), tx, session);
    (tmp, lifecycle_store, engine)
}

async fn create_author_run_node(engine: &mut WorkspaceEngine) -> String {
    engine
        .create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::AuthorRun,
            agent: Some(ProviderName::ClaudeCode),
            stage: WorkspaceStage::Running,
            round: None,
            title: "Story 生成".to_string(),
            summary: None,
            status: TimelineNodeStatus::Active,
        })
        .await
}

async fn create_reviewer_run_node(engine: &mut WorkspaceEngine) -> String {
    engine
        .create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::ReviewerRun,
            agent: Some(ProviderName::Codex),
            stage: WorkspaceStage::CrossReview,
            round: Some(1),
            title: "交叉审核 Round 1".to_string(),
            summary: None,
            status: TimelineNodeStatus::Active,
        })
        .await
}

#[tokio::test]
async fn stream_chunk_flushes_after_4kb_or_node_end() {
    let (_tmp, lifecycle_store, mut engine) = persistent_test_engine();
    let node_id = create_author_run_node(&mut engine).await;

    engine
        .buffer_stream_chunk(&node_id, "hello ".to_string())
        .await
        .unwrap();
    engine
        .buffer_stream_chunk(&node_id, "world".to_string())
        .await
        .unwrap();
    assert!(
        lifecycle_store
            .load_node_detail(&engine.session().session_id, &node_id)
            .is_err(),
        "small chunks should stay buffered before explicit flush"
    );

    engine.flush_stream_buffer(&node_id).await.unwrap();

    let detail = lifecycle_store
        .load_node_detail(&engine.session().session_id, &node_id)
        .unwrap();
    assert_eq!(detail.streaming_content, "hello world");

    let large = "x".repeat(4096);
    engine
        .buffer_stream_chunk(&node_id, large.clone())
        .await
        .unwrap();
    let detail = lifecycle_store
        .load_node_detail(&engine.session().session_id, &node_id)
        .unwrap();
    assert!(detail.streaming_content.ends_with(&large));
}

#[tokio::test]
async fn permission_request_and_response_are_persisted_to_node_detail() {
    let (_tmp, lifecycle_store, mut engine) = persistent_test_engine();
    let node_id = create_author_run_node(&mut engine).await;

    engine
        .persist_permission_request(
            &node_id,
            "permission_1".to_string(),
            serde_json::json!({"tool_name": "shell", "description": "cargo test"}),
        )
        .await
        .unwrap();
    engine
        .persist_permission_response(
            &node_id,
            "permission_1".to_string(),
            serde_json::json!({"approved": true, "reason": null}),
        )
        .await
        .unwrap();

    let detail = lifecycle_store
        .load_node_detail(&engine.session().session_id, &node_id)
        .unwrap();
    assert_eq!(detail.permission_events.len(), 1);
    assert_eq!(detail.permission_events[0].request_id, "permission_1");
    assert_eq!(
        detail.permission_events[0].response.as_ref().unwrap()["approved"],
        true
    );
}

#[tokio::test]
async fn permission_timeout_marks_node_detail_and_returns_to_prepare_context() {
    let (tmp, checkpoint_store) = setup();
    let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let (engine_tx, mut engine_rx) = mpsc::channel(64);
    let session_record = lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput { project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        entity_id: "story_spec_0001".to_string(),
        workspace_type: WorkspaceType::Story,
        author_provider: ProviderName::ClaudeCode,
        reviewer_provider: Some(ProviderName::Codex),

        review_rounds: 2,
        superpowers_enabled: true, openspec_enabled: true, work_item_plan_options: None, })
        .unwrap();
    let session = WorkspaceSession::from_record(session_record);
    let mut engine = WorkspaceEngine::new_persistent(
        checkpoint_store,
        lifecycle_store.clone(),
        engine_tx,
        session,
    );
    let node_id = create_author_run_node(&mut engine).await;
    engine.mark_active_run_started("run-1");
    engine
        .persist_permission_request(
            &node_id,
            "permission_1".to_string(),
            serde_json::json!({"tool_name": "shell", "description": "cargo test"}),
        )
        .await
        .unwrap();

    let (provider_event_tx, provider_event_rx) = mpsc::channel(8);
    let (provider_command_tx, _provider_command_rx) = mpsc::channel(8);
    provider_event_tx
        .send(ProviderEvent::PermissionTimeout {
            permission_id: "permission_1".to_string(),
        })
        .await
        .unwrap();
    drop(provider_event_tx);

    engine
        .drive_provider_session(ProviderSessionDriveInput {
            session: Ok(ProviderSession {
                native_session_id: None,
                events: provider_event_rx,
                commands: provider_command_tx,
            }),
            command_rx: empty_provider_commands(),
            node_id: Some(node_id.clone()),
            agent: Some(ProviderName::ClaudeCode),
            role: ProviderConversationRole::Author,
            artifact_retry: None,
            revision_resume_fallback: None,
        })
        .await;

    let detail = lifecycle_store
        .load_node_detail(&engine.session().session_id, &node_id)
        .unwrap();
    assert_eq!(
        detail.permission_events[0]
            .response
            .as_ref()
            .and_then(|value| value.get("status"))
            .and_then(|value| value.as_str()),
        Some("timeout")
    );
    assert_eq!(detail.status, TimelineNodeStatus::Failed);
    assert_eq!(engine.current_stage(), WorkspaceStage::PrepareContext);
    assert_eq!(engine.active_run_id(), None);

    let mut saw_timeout_event = false;
    while let Ok(event) = engine_rx.try_recv() {
        if let EngineEvent::PermissionTimeout {
            permission_id,
            node_id: event_node_id,
        } = event
        {
            saw_timeout_event = permission_id == "permission_1"
                && event_node_id.as_deref() == Some(node_id.as_str());
        }
    }
    assert!(saw_timeout_event);
}

#[tokio::test]
async fn verdict_and_artifact_ref_are_persisted_to_node_detail() {
    let (_tmp, lifecycle_store, mut engine) = persistent_test_engine();
    let node_id = create_reviewer_run_node(&mut engine).await;

    engine
        .persist_review_verdict(
            &node_id,
            serde_json::json!({"verdict": "pass", "summary": "ok"}),
        )
        .await
        .unwrap();
    engine
        .persist_artifact_ref(
            &node_id,
            ArtifactRef {
                artifact_id: "artifact_story_spec_0001".to_string(),
                version: 2,
            },
        )
        .await
        .unwrap();

    let detail = lifecycle_store
        .load_node_detail(&engine.session().session_id, &node_id)
        .unwrap();
    assert_eq!(detail.verdict.as_ref().unwrap()["verdict"], "pass");
    assert_eq!(detail.artifact_ref.as_ref().unwrap().version, 2);
}

#[tokio::test]
async fn complete_review_persists_structured_output_diagnostic() {
    let (_tmp, lifecycle_store, mut engine) = persistent_test_engine();
    let node_id = create_reviewer_run_node(&mut engine).await;
    let completion = crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
        "review without requested structured output",
        None,
    );
    let error = engine
        .parse_review_completion_for_active_node(&completion)
        .expect_err("plain completion should not be trusted as a review verdict");
    let verdict = fallback_review_verdict(&completion, &error, false);

    engine.complete_review(completion, verdict).await;

    let detail = lifecycle_store
        .load_node_detail(&engine.session().session_id, &node_id)
        .unwrap();
    let diagnostic = &detail.verdict.as_ref().unwrap()["structured_output_diagnostic"];
    assert_eq!(diagnostic["code"], "structured_output_not_requested");
    assert_eq!(diagnostic["repair_attempted"], false);
    assert_eq!(diagnostic["repair_succeeded"], false);
}

// 退役留档（T5/REQ-RET-02）：`handle_user_message_transitions_from_prepare_to_running` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

#[tokio::test]
async fn empty_start_generation_records_default_prompt_for_audit() {
    let (_tmp, lifecycle_store, mut engine) = persistent_test_engine();

    engine
        .handle_user_message(
            String::new(),
            Arc::new(FakeStreamingProvider),
            empty_provider_commands(),
        )
        .await;

    let user_message = engine
        .session()
        .messages
        .iter()
        .find(|message| message.role == "user")
        .expect("user prompt message");
    assert!(!user_message.content.trim().is_empty());
    assert!(user_message.content.contains("Story Spec"));

    let author_node = engine
        .timeline_nodes
        .iter()
        .find(|node| node.node_type == TimelineNodeType::AuthorRun)
        .expect("author run node");
    let detail = lifecycle_store
        .load_node_detail(&engine.session().session_id, &author_node.node_id)
        .expect("author run detail");
    let prompt = detail.prompt.as_ref().expect("prompt snapshot");
    assert!(prompt.contains("Workspace 类型: Story Spec"));
    assert!(prompt.contains(&user_message.content));
}
