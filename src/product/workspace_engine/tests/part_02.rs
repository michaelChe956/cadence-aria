#[test]
fn artifact_constraint_spec_defines_story_required_and_forbidden_rules() {
    let spec = artifact_constraint_spec_for(&WorkspaceType::Story);

    assert!(
        spec.required_headings
            .iter()
            .any(|rule| rule.label == "功能需求")
    );
    assert!(
        spec.required_headings
            .iter()
            .any(|rule| rule.label == "成功标准")
    );
    assert!(
        spec.required_id_patterns
            .iter()
            .any(|rule| rule.label == "[REQ-*]")
    );
    assert!(
        spec.required_id_patterns
            .iter()
            .any(|rule| rule.label == "[AC-*]")
    );
    assert!(
        spec.forbidden_headings
            .iter()
            .any(|rule| rule.label == "Work Items")
    );
    assert!(
        spec.forbidden_headings
            .iter()
            .any(|rule| rule.label == "任务拆分")
    );
    assert!(
        spec.forbidden_tokens
            .iter()
            .any(|rule| rule.label == "[TASK-*]")
    );
    assert!(
        spec.reviewer_must_fix_rules
            .iter()
            .any(|rule| rule.contains("must_fix") && rule.contains("Story"))
    );
}

#[test]
fn story_artifact_constraint_report_rejects_work_item_leakage() {
    let report = validate_workspace_artifact_constraints(
        "# Story Spec\n\n\
         ## 范围\n覆盖基础流程。\n\n\
         ## 用户故事\n作为用户，我要完成操作。\n\n\
         ## 功能需求\n- [REQ-001] 系统支持操作。\n\n\
         ## 成功标准\n- [AC-001] 操作成功。\n\n\
         ## 待确认项\n无。\n\n\
         ## 非功能需求\n无。\n\n\
         ## Work Items\n- [TASK-001] 实现后端。\n",
        &WorkspaceType::Story,
    );

    assert!(!report.passed);
    assert!(
        report
            .forbidden_headings
            .iter()
            .any(|heading| heading.contains("Work Items"))
    );
    assert!(
        report
            .forbidden_tokens
            .iter()
            .any(|token| token.contains("[TASK-001]"))
    );
    assert!(
        !content_has_complete_workspace_artifact(
            "# Story Spec\n\n## 功能需求\n- [REQ-001] A\n\n## 成功标准\n- [AC-001] B\n\n## Work Items\n- [TASK-001] C",
            &WorkspaceType::Story,
        ),
        "compat wrapper should reject forbidden Story leakage"
    );
}

#[test]
fn work_item_plan_constraints_allow_task_ids() {
    let report = validate_workspace_artifact_constraints(
        "# Work Item Plan\n\n\
         ## 计划范围\n本计划覆盖 Issue。\n\n\
         ## 任务拆分\n- [TASK-001] 后端。\n\n\
         ## 依赖图\n无。\n\n\
         ## 验证计划\ncargo test --locked。\n\n\
         ## 执行顺序\n先后端。\n\n\
         ## 风险\n无。\n\n\
         ## 追踪关系\nsource ids: Story Spec story_spec_0001, Design Spec design_spec_0001。\n\
         [TASK-001] -> [REQ-001]\n",
        &WorkspaceType::WorkItemPlan,
    );

    assert!(report.passed, "{report:?}");
}

#[test]
fn work_item_artifact_constraint_report_rejects_sibling_task_split() {
    let report = validate_workspace_artifact_constraints(
        "# Work Item\n\n\
         ## 目标\n实现当前任务。\n\n\
         ## 范围\n仅当前任务。\n\n\
         ## 实现步骤\n- 接入接口。\n\n\
         ## 依赖\n无。\n\n\
         ## 验证命令\ncargo test --locked --lib current_task。\n\n\
         ## 风险\n无。\n\n\
         ## 追踪关系\n[REQ-001]\n\n\
         ## 任务拆分\n- [TASK-001] 后端。\n- [TASK-002] 前端。\n",
        &WorkspaceType::WorkItem,
    );

    assert!(!report.passed);
    assert!(
        report
            .forbidden_headings
            .iter()
            .any(|heading| heading.contains("任务拆分")),
        "{report:?}"
    );
    assert!(
        report
            .forbidden_tokens
            .iter()
            .any(|token| token.contains("[TASK-001]") && token.contains("[TASK-002]")),
        "{report:?}"
    );
}

#[test]
fn workspace_artifact_gate_is_enabled_for_markdown_workspace_types() {
    for workspace_type in [
        WorkspaceType::Story,
        WorkspaceType::Design,
        WorkspaceType::WorkItem,
        WorkspaceType::WorkItemPlan,
    ] {
        let (event_tx, _event_rx) = mpsc::channel(8);
        let mut session = make_session(&format!("sess_artifact_gate_{workspace_type:?}"));
        session.workspace_type = workspace_type.clone();
        let checkpoint_tmp = TempDir::new().unwrap();
        let engine = WorkspaceEngine::new(
            Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
            event_tx,
            session,
        );

        assert!(
            engine.workspace_requires_artifact_gate(),
            "{workspace_type:?} should use workspace artifact gate"
        );
    }
}

#[test]
fn workspace_provider_inputs_use_three_hour_timeout() {
    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut session = make_session("sess_workspace_timeout");
    session.artifact = Some(artifact_payload(
        "# Story Spec\n\n## 功能需求\n- [REQ-001] Draft.\n",
    ));
    let checkpoint_tmp = TempDir::new().unwrap();
    let mut engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
        event_tx,
        session,
    );
    engine.latest_review_verdict = Some(ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: "补充验收标准".to_string(),
        summary: "需要返修".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
    });

    assert_eq!(
        engine
            .build_streaming_input("开始生成", AuthorPromptMode::FullConversation)
            .expect("author input")
            .timeout_secs,
        10_800
    );
    assert_eq!(
        engine
            .build_review_input()
            .expect("review input")
            .timeout_secs,
        10_800
    );
    assert_eq!(
        engine
            .build_revision_input()
            .expect("revision input")
            .timeout_secs,
        10_800
    );
}

#[test]
fn review_input_keeps_current_artifact_and_context_without_old_assistant_artifacts() {
    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut session = make_session("sess_review_prompt_dedupe");
    session.messages = vec![
        SessionMessage {
            id: "msg_001".to_string(),
            role: "system".to_string(),
            content: "系统上下文：真实 issue 描述。".to_string(),
            checkpoint_id: None,
            created_at: "2026-06-01T00:00:00Z".to_string(),
        },
        SessionMessage {
            id: "msg_002".to_string(),
            role: "user".to_string(),
            content: "用户补充：必须覆盖 n=10 -> 89。".to_string(),
            checkpoint_id: None,
            created_at: "2026-06-01T00:00:01Z".to_string(),
        },
        SessionMessage {
            id: "msg_003".to_string(),
            role: "assistant".to_string(),
            content: "# Old Story Spec\n\n## 功能需求\n- [REQ-OLD] 旧稿。\n\n## 成功标准\n- [AC-OLD] 旧验收。\n".to_string(),
            checkpoint_id: None,
            created_at: "2026-06-01T00:00:02Z".to_string(),
        },
    ];
    session.artifact = Some(artifact_payload(
        "# Current Story Spec\n\n## 功能需求\n- [REQ-001] 当前稿。\n\n## 成功标准\n- [AC-001] 当前稿覆盖 n=10 -> 89。\n",
    ));
    let checkpoint_tmp = TempDir::new().unwrap();
    let engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
        event_tx,
        session,
    );

    let input = engine.build_review_input().expect("review input");

    assert!(input.prompt.contains("系统上下文：真实 issue 描述。"));
    assert!(input.prompt.contains("用户补充：必须覆盖 n=10 -> 89。"));
    assert_eq!(input.prompt.matches("# Current Story Spec").count(), 1);
    assert!(
        !input.prompt.contains("# Old Story Spec"),
        "review prompt should not include historical assistant artifact bodies: {}",
        input.prompt
    );
    assert!(
        input
            .prompt
            .contains("{\"verdict\":\"pass|revise|needs_human\"")
    );
}

#[test]
fn review_input_marks_design_artifact_as_extracted_markdown_without_outer_fence() {
    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut session = make_session("sess_design_review_prompt_extracted_artifact");
    session.workspace_type = WorkspaceType::Design;
    session.artifact = Some(artifact_payload(
        "# 底层依赖安装任务 Design Spec\n\n\
         ## 设计范围\n\n\
         - [DEC-001] 覆盖依赖安装任务。\n\n\
         ## API 契约\n\n\
         ```json\n\
         {\"task_id\":\"install_001\"}\n\
         ```\n\n\
         ## 风险\n\n\
         - 无。\n",
    ));
    let checkpoint_tmp = TempDir::new().unwrap();
    let engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
        event_tx,
        session,
    );

    let input = engine.build_review_input().expect("review input");

    assert!(
        input
            .prompt
            .contains("当前已提取 Artifact Markdown（daemon 已剥离外层 artifact fence）"),
        "review prompt should label stored artifact as extracted markdown: {}",
        input.prompt
    );
    assert!(input.prompt.contains("# 底层依赖安装任务 Design Spec"));
    assert!(
        input
            .prompt
            .contains("不要因为当前 Artifact 未包含外层 artifact fence 判定返修"),
        "reviewer should not reject extracted artifact for missing outer fence: {}",
        input.prompt
    );
}

#[test]
fn review_input_injects_artifact_boundary_must_fix_rules_for_workspace_types() {
    for (workspace_type, expected_rule) in [
        (
            WorkspaceType::Story,
            "Story artifact: Work Item heading, task splitting, [TASK-*], or WI-* content must be reported as must_fix.",
        ),
        (
            WorkspaceType::Design,
            "Design artifact: Work Item Plan, development task list, task splitting, or execution checklist content must be reported as must_fix.",
        ),
        (
            WorkspaceType::WorkItem,
            "Work Item artifact: sibling tasks, issue-level full plans, or cross-task content must be reported as must_fix.",
        ),
    ] {
        let (event_tx, _event_rx) = mpsc::channel(8);
        let mut session = make_session(&format!("sess_review_boundary_{workspace_type:?}"));
        session.workspace_type = workspace_type.clone();
        session.artifact = Some(artifact_payload("# Artifact\n\n## 内容\n待审核。\n"));
        let checkpoint_tmp = TempDir::new().unwrap();
        let engine = WorkspaceEngine::new(
            Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
            event_tx,
            session,
        );

        let input = engine.build_review_input().expect("review input");

        assert!(
            input.prompt.contains("[artifact_boundary_must_fix_rules]"),
            "review prompt should include boundary rule section for {workspace_type:?}: {}",
            input.prompt
        );
        assert!(
            input.prompt.contains(expected_rule),
            "review prompt should include type-specific must_fix rule for {workspace_type:?}: {}",
            input.prompt
        );
    }
}

fn persistent_test_engine() -> (TempDir, LifecycleStore, WorkspaceEngine) {
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
async fn handle_user_message_transitions_from_prepare_to_running() {
    let (_tmp, store) = setup();
    let (tx, mut rx) = mpsc::channel(64);
    let session = make_session("sess_001");
    let mut engine = WorkspaceEngine::new(store, tx, session);

    engine
        .handle_user_message(
            "hello world".to_string(),
            Arc::new(FakeStreamingProvider),
            empty_provider_commands(),
        )
        .await;

    engine
        .handle_author_decision(AuthorDecision::Accept)
        .await
        .unwrap();

    let mut saw_running = false;
    while let Ok(event) = rx.try_recv() {
        if matches!(event, EngineEvent::StageChange { stage } if stage == "running") {
            saw_running = true;
        }
    }
    assert!(saw_running);
    assert_eq!(engine.session().stage, WorkspaceStage::CrossReview);
    assert_eq!(engine.session().messages.len(), 2); // user + assistant
    assert_eq!(engine.session().messages[0].role, "user");
    assert_eq!(engine.session().messages[1].role, "assistant");
    assert!(engine.session().messages[1].checkpoint_id.is_some());

    match engine.build_session_state() {
        WsOutMessage::SessionState {
            timeline_nodes,
            active_node_id,
            ..
        } => {
            assert!(
                timeline_nodes.iter().any(|node| {
                    node.node_type == TimelineNodeType::AuthorRun
                        && node.status == TimelineNodeStatus::Completed
                }),
                "generation node should be completed"
            );
            let active_id = active_node_id.expect("active review node id");
            let active = timeline_nodes
                .iter()
                .find(|node| node.node_id == active_id)
                .expect("active timeline node");
            assert_eq!(active.node_type, TimelineNodeType::ReviewerRun);
            assert_eq!(active.agent, Some(ProviderName::Codex));
            assert_eq!(active.status, TimelineNodeStatus::Active);
        }
        _ => panic!("expected SessionState"),
    }
}

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

#[tokio::test]
async fn fake_reviewer_creates_skipped_review_node_and_enters_human_confirm() {
    let (_tmp, store) = setup();
    let (tx, _) = mpsc::channel(64);
    let mut session = make_session("sess_fake_review");
    session.reviewer_provider = Some(ProviderName::Fake);
    let mut engine = WorkspaceEngine::new(store, tx, session);

    engine
        .handle_user_message(
            "hello world".to_string(),
            Arc::new(FakeStreamingProvider),
            empty_provider_commands(),
        )
        .await;

    engine
        .handle_author_decision(AuthorDecision::Accept)
        .await
        .unwrap();

    assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
    match engine.build_session_state() {
        WsOutMessage::SessionState { timeline_nodes, .. } => {
            assert!(timeline_nodes.iter().any(|node| {
                node.node_type == TimelineNodeType::ReviewerRun
                    && node.status == TimelineNodeStatus::Skipped
                    && node.summary.as_deref() == Some("未执行真实 review（Fake 快速路径）")
            }));
        }
        _ => panic!("expected SessionState"),
    }
}
