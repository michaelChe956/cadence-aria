#[async_trait::async_trait]
impl StreamingProviderAdapter for RevisionResumeStallThenSuccessProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.inputs.lock().unwrap().push(input);
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        let call_no = *calls;
        drop(calls);

        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        let output = self.output.to_string();
        tokio::spawn(async move {
            if call_no == 1 {
                let _ = event_tx
                    .send(ProviderEvent::Failed {
                        message:
                            "Codex resume stalled before provider progress for thread codex-stale-ephemeral-thread"
                                .to_string(),
                    })
                    .await;
            } else {
                let _ = event_tx
                    .send(ProviderEvent::Completed {
                        full_output: output,
                        provider_session_id: Some("codex-fresh-thread".to_string()),
                    })
                    .await;
            }
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

#[async_trait::async_trait]
impl StreamingProviderAdapter for RevisionInputRecordingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        *self.input.lock().unwrap() = Some(input);
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        let output = self.output.to_string();
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Completed {
                    full_output: output,
                    provider_session_id: Some("provider-author-session-1".to_string()),
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
async fn handle_rollback_truncates_messages() {
    let (_tmp, store) = setup();
    let (tx, _) = mpsc::channel(64);
    let session = make_session("sess_002");
    let mut engine = WorkspaceEngine::new(store, tx, session);

    engine
        .handle_user_message(
            "first".to_string(),
            Arc::new(FakeStreamingProvider),
            empty_provider_commands(),
        )
        .await;
    engine
        .handle_user_message(
            "second".to_string(),
            Arc::new(FakeStreamingProvider),
            empty_provider_commands(),
        )
        .await;

    assert_eq!(engine.session().messages.len(), 4);

    let cp_id = engine.session().messages[1].checkpoint_id.clone().unwrap();
    engine.handle_rollback(&cp_id).await.unwrap();

    assert_eq!(engine.session().messages.len(), 2);
}

#[tokio::test]
async fn handle_confirm_transitions_stage() {
    let (_tmp, store) = setup();
    let (tx, _) = mpsc::channel(64);
    let mut session = make_session("sess_003");
    session.stage = WorkspaceStage::HumanConfirm;
    let mut engine = WorkspaceEngine::new(store, tx, session);

    engine.handle_confirm().await.unwrap();
    assert_eq!(engine.session().stage, WorkspaceStage::Completed);
}

#[tokio::test]
async fn handle_confirm_completes_human_confirm_node_before_completed_node() {
    let (_tmp, store) = setup();
    let (tx, _) = mpsc::channel(64);
    let mut session = make_session("sess_confirm_timeline");
    session.reviewer_provider = Some(ProviderName::Fake);
    let mut engine = WorkspaceEngine::new(store, tx, session);

    engine
        .handle_user_message(
            "start".to_string(),
            Arc::new(FakeStreamingProvider),
            empty_provider_commands(),
        )
        .await;
    engine
        .handle_author_decision(AuthorDecision::Accept)
        .await
        .unwrap();
    assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);

    engine.handle_confirm().await.unwrap();

    match engine.build_session_state() {
        WsOutMessage::SessionState {
            timeline_nodes,
            active_node_id,
            stage,
            ..
        } => {
            assert_eq!(stage, "completed");
            assert!(timeline_nodes.iter().any(|node| {
                node.node_type == TimelineNodeType::HumanConfirm
                    && node.status == TimelineNodeStatus::Completed
            }));
            let active = timeline_nodes
                .iter()
                .find(|node| Some(&node.node_id) == active_node_id.as_ref())
                .expect("active completed node");
            assert_eq!(active.node_type, TimelineNodeType::Completed);
            assert_eq!(active.status, TimelineNodeStatus::Completed);
            assert_eq!(
                active_timeline_node_id(&timeline_nodes).as_deref(),
                active_node_id.as_deref()
            );
        }
        _ => panic!("expected SessionState"),
    }
}

#[test]
fn active_timeline_node_id_prefers_terminal_completed_node_over_stale_active_node() {
    let session = make_session("sess_stale_timeline");
    let provider_config_snapshot = ProviderConfigSnapshot {
        author: session.author_provider.clone(),
        reviewer: session.reviewer_provider.clone(),
        review_rounds: session.review_rounds,
    };
    let stale_human_confirm = TimelineNode {
        node_id: "timeline_node_001".to_string(),
        node_type: TimelineNodeType::HumanConfirm,
        agent: None,
        stage: WsWorkspaceStage::HumanConfirm,
        round: None,
        status: TimelineNodeStatus::Active,
        title: "人工确认".to_string(),
        summary: Some("等待人工确认".to_string()),
        started_at: "2026-05-19T00:00:00Z".to_string(),
        completed_at: None,
        duration_ms: None,
        artifact_ref: Some("artifact_current".to_string()),
        provider_config_snapshot: provider_config_snapshot.clone(),
        retry: None,
    };
    let completed = TimelineNode {
        node_id: "timeline_node_002".to_string(),
        node_type: TimelineNodeType::Completed,
        agent: None,
        stage: WsWorkspaceStage::Completed,
        round: None,
        status: TimelineNodeStatus::Completed,
        title: "流程完成".to_string(),
        summary: Some("已确认通过".to_string()),
        started_at: "2026-05-19T00:01:00Z".to_string(),
        completed_at: None,
        duration_ms: None,
        artifact_ref: Some("artifact_current".to_string()),
        provider_config_snapshot,
        retry: None,
    };

    assert_eq!(
        active_timeline_node_id(&[stale_human_confirm, completed]).as_deref(),
        Some("timeline_node_002")
    );
}

#[tokio::test]
async fn persistent_engine_keeps_open_stage_after_failed_running_node() {
    let (tmp, checkpoint_store) = setup();
    let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let (tx, _) = mpsc::channel(64);
    let session_record = lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "story_spec_0001".to_string(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .unwrap();
    let session_id = session_record.id.clone();
    let provider_config_snapshot = ProviderConfigSnapshot {
        author: session_record.author_provider.clone(),
        reviewer: Some(session_record.reviewer_provider.clone()),
        review_rounds: session_record.review_rounds,
    };
    lifecycle_store
        .save_timeline_nodes(
            &session_id,
            &[
                TimelineNode {
                    node_id: "timeline_node_001".to_string(),
                    node_type: TimelineNodeType::StartGeneration,
                    agent: None,
                    stage: WsWorkspaceStage::PrepareContext,
                    round: None,
                    status: TimelineNodeStatus::Completed,
                    title: "开始生成".to_string(),
                    summary: None,
                    started_at: "2026-06-01T14:12:29Z".to_string(),
                    completed_at: Some("2026-06-01T14:12:29Z".to_string()),
                    duration_ms: Some(0),
                    artifact_ref: None,
                    provider_config_snapshot: provider_config_snapshot.clone(),
                    retry: None,
                },
                TimelineNode {
                    node_id: "timeline_node_002".to_string(),
                    node_type: TimelineNodeType::AuthorRun,
                    agent: Some(ProviderName::ClaudeCode),
                    stage: WsWorkspaceStage::Running,
                    round: None,
                    status: TimelineNodeStatus::Failed,
                    title: "Story Spec 生成".to_string(),
                    summary: Some("运行已中止".to_string()),
                    started_at: "2026-06-01T14:12:29Z".to_string(),
                    completed_at: Some("2026-06-01T14:12:36Z".to_string()),
                    duration_ms: None,
                    artifact_ref: None,
                    provider_config_snapshot,
                    retry: None,
                },
            ],
        )
        .unwrap();

    let session = WorkspaceSession::from_record(
        lifecycle_store
            .get_workspace_session(&session_id)
            .expect("workspace session"),
    );
    let engine = WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store, tx, session);

    assert_eq!(engine.current_stage(), WorkspaceStage::PrepareContext);
    match engine.build_session_state() {
        WsOutMessage::SessionState { stage, .. } => {
            assert_eq!(stage, "prepare_context");
        }
        other => panic!("expected session_state, got {other:?}"),
    }
}

#[tokio::test]
async fn build_session_state_returns_correct_structure() {
    let (_tmp, store) = setup();
    let (tx, _) = mpsc::channel(64);
    let session = make_session("sess_004");
    let engine = WorkspaceEngine::new(store, tx, session);

    let state = engine.build_session_state();
    match state {
        WsOutMessage::SessionState {
            session_id, stage, ..
        } => {
            assert_eq!(session_id, "sess_004");
            assert_eq!(stage, "prepare_context");
        }
        _ => panic!("expected SessionState"),
    }
}

#[tokio::test]
async fn build_session_state_includes_node_details_and_active_run_id() {
    let (tmp, checkpoint_store) = setup();
    let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let (tx, _) = mpsc::channel(64);
    let session_record = lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "issue_work_item_plan_0001".to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 2,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .unwrap();
    let session = WorkspaceSession::from_record(session_record);
    let session_id = session.session_id.clone();
    let mut engine =
        WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store.clone(), tx, session);
    engine.timeline_nodes.push(TimelineNode {
        node_id: "node-1".to_string(),
        node_type: TimelineNodeType::AuthorRun,
        agent: Some(ProviderName::ClaudeCode),
        stage: WsWorkspaceStage::Completed,
        round: None,
        status: TimelineNodeStatus::Completed,
        title: "生成".to_string(),
        summary: None,
        started_at: "2026-05-20T14:30:00Z".to_string(),
        completed_at: Some("2026-05-20T14:35:00Z".to_string()),
        duration_ms: Some(300000),
        artifact_ref: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::ClaudeCode,
            reviewer: None,
            review_rounds: 0,
        },
        retry: None,
    });
    let huge_prompt = "P".repeat(3000);
    let huge_stream = "S".repeat(3000);
    let huge_output = "O".repeat(3000);
    let artifact_markdown = format!("# Artifact\n\n{}", "M".repeat(3000));
    engine.artifact_versions.push(ArtifactVersion {
        version: 2,
        payload: ArtifactPayload::Markdown {
            markdown: artifact_markdown.clone(),
            diff: None,
        },
        generated_by: ProviderName::ClaudeCode,
        reviewed_by: Some(ProviderName::Codex),
        review_verdict: Some(ReviewVerdictType::Pass),
        confirmed_by: Some("user".to_string()),
        is_current: true,
        created_at: "2026-05-20T14:35:00Z".to_string(),
        source_node_id: "node-1".to_string(),
    });
    let detail = NodeDetail {
        node_id: "node-1".to_string(),
        session_id: session_id.clone(),
        node_type: TimelineNodeType::AuthorRun,
        status: TimelineNodeStatus::Completed,
        agent_role: Some(AgentRole::Author),
        provider: Some(ProviderSnapshot {
            name: "claude_code".to_string(),
            model: "claude-opus-4-7".to_string(),
        }),
        prompt: Some(huge_prompt.clone()),
        messages: vec![],
        streaming_content: huge_stream.clone(),
        execution_events: vec![serde_json::json!({
            "event_id": "call_read",
            "kind": "command",
            "status": "completed",
            "title": "Command completed",
            "detail": "exit code 0",
            "command": "sed -n '1,120p' src/lib.rs",
            "cwd": "/repo",
            "output": huge_output,
            "exit_code": 0
        })],
        permission_events: vec![PermissionEvent {
            request_id: "perm-1".to_string(),
            request: serde_json::json!({"tool": "shell"}),
            response: Some(serde_json::json!({"approved": true})),
            ts: "2026-05-20T14:31:00Z".to_string(),
        }],
        verdict: None,
        artifact_ref: Some(ArtifactRef {
            artifact_id: "artifact-1".to_string(),
            version: 2,
        }),
        is_revision: false,
        base_artifact_ref: None,
        started_at: "2026-05-20T14:30:00Z".to_string(),
        ended_at: Some("2026-05-20T14:35:00Z".to_string()),
    };
    lifecycle_store
        .save_node_detail(&session_id, "node-1", &detail)
        .unwrap();
    engine.mark_active_run_started("run-1");

    let state = engine.build_session_state();
    let serialized = serde_json::to_string(&state).unwrap();
    match state {
        WsOutMessage::SessionState {
            timeline_node_details,
            timeline_node_summaries,
            artifact_versions,
            artifact_version_summaries,
            active_run_id,
            ..
        } => {
            assert!(artifact_versions.is_empty());

            let inline_detail = timeline_node_details
                .get("node-1")
                .expect("inline node detail");
            assert_eq!(inline_detail.node_id, "node-1");
            assert_eq!(inline_detail.prompt, None);
            assert!(inline_detail.messages.is_empty());
            assert!(inline_detail.streaming_content.chars().count() <= SUMMARY_PREVIEW_CHARS);
            assert_ne!(inline_detail.streaming_content, huge_stream);
            assert_eq!(inline_detail.execution_events.len(), 1);
            assert_eq!(
                inline_detail.execution_events[0]
                    .get("event_id")
                    .and_then(serde_json::Value::as_str),
                Some("call_read")
            );
            assert_eq!(
                inline_detail.execution_events[0]
                    .get("command")
                    .and_then(serde_json::Value::as_str),
                Some("sed -n '1,120p' src/lib.rs")
            );
            assert!(
                inline_detail.execution_events[0]
                    .get("output")
                    .is_some_and(serde_json::Value::is_null)
            );
            assert!(inline_detail.permission_events.is_empty());
            assert_eq!(inline_detail.artifact_ref.as_ref().unwrap().version, 2);

            let summary = timeline_node_summaries.get("node-1").expect("node summary");
            assert_eq!(summary.node_id, "node-1");
            assert_eq!(summary.prompt_size, huge_prompt.len());
            assert!(summary.prompt_preview.as_ref().unwrap().chars().count() <= 2048);
            assert_ne!(
                summary.prompt_preview.as_deref(),
                Some(huge_prompt.as_str())
            );
            assert_eq!(summary.stream_size, huge_stream.len());
            assert!(summary.stream_preview.as_ref().unwrap().chars().count() <= 2048);
            assert_ne!(
                summary.stream_preview.as_deref(),
                Some(huge_stream.as_str())
            );
            assert_eq!(summary.execution_event_count, 1);
            assert_eq!(summary.artifact_ref.as_deref(), Some("artifact-1/v2"));
            assert!(summary.has_large_outputs);

            let artifact_summary = artifact_version_summaries
                .iter()
                .find(|summary| summary.version == 2)
                .expect("artifact summary");
            assert_eq!(artifact_summary.markdown_size, artifact_markdown.len());
            assert!(artifact_summary.markdown_preview.chars().count() <= 2048);
            assert_ne!(artifact_summary.markdown_preview, artifact_markdown);
            assert_eq!(active_run_id.as_deref(), Some("run-1"));
        }
        _ => panic!("expected SessionState"),
    }
    assert!(!serialized.contains(&huge_prompt));
    assert!(!serialized.contains(&huge_stream));
    assert!(!serialized.contains(&huge_output));
    assert!(!serialized.contains(&artifact_markdown));
}

#[tokio::test]
async fn build_session_state_keeps_story_details_out_of_inline_payload() {
    let (tmp, checkpoint_store) = setup();
    let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let (tx, _) = mpsc::channel(64);
    let session_record = lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "story_spec_0001".to_string(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 2,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .unwrap();
    let session = WorkspaceSession::from_record(session_record);
    let session_id = session.session_id.clone();
    let mut engine =
        WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store.clone(), tx, session);
    engine.timeline_nodes.push(TimelineNode {
        node_id: "node-story".to_string(),
        node_type: TimelineNodeType::AuthorRun,
        agent: Some(ProviderName::ClaudeCode),
        stage: WsWorkspaceStage::Completed,
        round: None,
        status: TimelineNodeStatus::Completed,
        title: "Story 生成".to_string(),
        summary: None,
        started_at: "2026-05-20T14:30:00Z".to_string(),
        completed_at: Some("2026-05-20T14:35:00Z".to_string()),
        duration_ms: Some(300000),
        artifact_ref: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::ClaudeCode,
            reviewer: None,
            review_rounds: 0,
        },
        retry: None,
    });
    lifecycle_store
        .save_node_detail(
            &session_id,
            "node-story",
            &NodeDetail {
                node_id: "node-story".to_string(),
                session_id: session_id.clone(),
                node_type: TimelineNodeType::AuthorRun,
                status: TimelineNodeStatus::Completed,
                agent_role: Some(AgentRole::Author),
                provider: None,
                prompt: None,
                messages: vec![],
                streaming_content: "Story provider stream".to_string(),
                execution_events: vec![],
                permission_events: vec![],
                verdict: None,
                artifact_ref: None,
                is_revision: false,
                base_artifact_ref: None,
                started_at: "2026-05-20T14:30:00Z".to_string(),
                ended_at: Some("2026-05-20T14:35:00Z".to_string()),
            },
        )
        .unwrap();

    match engine.build_session_state() {
        WsOutMessage::SessionState {
            timeline_node_details,
            timeline_node_summaries,
            ..
        } => {
            assert!(timeline_node_details.is_empty());
            assert!(
                timeline_node_summaries
                    .get("node-story")
                    .and_then(|summary| summary.stream_preview.as_deref())
                    .is_some_and(|stream| stream.contains("Story provider stream"))
            );
        }
        _ => panic!("expected SessionState"),
    }
}

#[tokio::test]
async fn append_active_run_stream_sends_event_when_detail_persist_fails() {
    let (tmp, checkpoint_store) = setup();
    let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let (tx, mut rx) = mpsc::channel(64);
    let session_record = lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "issue_work_item_plan_0001".to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 2,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .unwrap();
    let session = WorkspaceSession::from_record(session_record);
    let mut engine =
        WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store, tx, session);
    engine.active_node_id = Some("missing-node".to_string());

    let result = engine
        .append_active_run_stream("assistant", "正在生成 Work Item Plan：准备上下文\n")
        .await;

    assert!(result.is_err());
    let event = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("stream chunk event timeout")
        .expect("stream chunk event");
    match event {
        EngineEvent::StreamChunk {
            role,
            content,
            node_id,
        } => {
            assert_eq!(role, "assistant");
            assert!(content.contains("正在生成 Work Item Plan"));
            assert_eq!(node_id.as_deref(), Some("missing-node"));
        }
        _ => panic!("expected StreamChunk"),
    }
}

#[tokio::test]
async fn append_context_note_creates_timeline_node() {
    let (_tmp, store) = setup();
    let (tx, _) = mpsc::channel(64);
    let session = make_session("sess_context_note");
    let mut engine = WorkspaceEngine::new(store, tx, session);

    let node = engine
        .append_context_note("补充上下文".to_string())
        .await
        .unwrap();

    assert_eq!(node.node_type, TimelineNodeType::ContextNote);
    assert_eq!(node.status, TimelineNodeStatus::Completed);
    assert_eq!(node.summary.as_deref(), Some("补充上下文"));
    assert!(
        engine
            .timeline_nodes
            .iter()
            .any(|candidate| candidate.node_id == node.node_id)
    );
}
