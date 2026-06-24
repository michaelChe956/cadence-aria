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

#[test]
fn parse_review_verdict_reads_json_contract_from_tail_block() {
    let output = "整体可用，但需要补充异常路径。\n\n```json\n{\"verdict\":\"revise\",\"summary\":\"补充异常路径\"}\n```";

    let verdict = WorkspaceEngine::parse_review_verdict(output);

    assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
    assert_eq!(verdict.review_gate, ReviewGate::UserTriageRequired);
    assert_eq!(verdict.summary, "补充异常路径");
    assert_eq!(verdict.comments.trim(), "整体可用，但需要补充异常路径。");
}

#[test]
fn reviewer_prompt_requires_nonce_sentinel() {
    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut session = make_session("sess_reviewer_nonce_prompt");
    session.artifact = Some(artifact_payload(
        "# Story Spec\n\n## 功能需求\n- [REQ-001] Draft.",
    ));
    session.reviewer_provider = Some(ProviderName::Codex);
    let checkpoint_tmp = TempDir::new().unwrap();
    let engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
        event_tx,
        session,
    );

    let input = engine.build_review_input().expect("review input");

    assert!(input.prompt.contains("<ARIA_STRUCTURED_OUTPUT nonce=\""));
    assert!(input.prompt.contains("</ARIA_STRUCTURED_OUTPUT nonce=\""));
    assert!(input.prompt.contains("不得使用 Markdown code fence"));
    assert!(!input.prompt.contains("```json"));
}

#[test]
fn extract_structured_json_prefers_last_matching_nonce_block() {
    let output = "第一次输出\n\
        <ARIA_STRUCTURED_OUTPUT nonce=\"old00001\">{\"verdict\":\"needs_human\",\"summary\":\"old\"}</ARIA_STRUCTURED_OUTPUT nonce=\"old00001\">\n\
        最终输出\n\
        <ARIA_STRUCTURED_OUTPUT nonce=\"new00002\">{\"verdict\":\"pass\",\"summary\":\"new\"}</ARIA_STRUCTURED_OUTPUT nonce=\"new00002\">";

    let (comments, json) = extract_structured_json(output).expect("structured json");

    assert!(comments.contains("最终输出"));
    assert!(json.contains("\"summary\":\"new\""));
}

#[test]
fn extract_structured_json_ignores_nonce_mismatch() {
    let output = "review text\n\
        <ARIA_STRUCTURED_OUTPUT nonce=\"a1b2c3d4\">{\"verdict\":\"pass\",\"summary\":\"ok\"}</ARIA_STRUCTURED_OUTPUT nonce=\"deadbeef\">";

    assert!(extract_structured_json(output).is_none());
}

#[test]
fn extract_structured_json_falls_back_to_markdown_fence() {
    let output = "review text\n\n```json\n{\"verdict\":\"pass\",\"summary\":\"ok\"}\n```";

    let (comments, json) = extract_structured_json(output).expect("markdown fallback json");

    assert_eq!(comments.trim(), "review text");
    assert!(json.contains("\"summary\":\"ok\""));
}

#[test]
fn extract_structured_json_treats_non_nonce_sentinel_as_text() {
    let output =
        "review text\n<ARIA_STRUCTURED_OUTPUT>{\"verdict\":\"pass\"}</ARIA_STRUCTURED_OUTPUT>";

    assert!(extract_structured_json(output).is_none());
}

#[test]
fn parse_review_verdict_does_not_upgrade_actionable_comments_without_strong_findings() {
    let output = "**审核结论**\n\n\
        不建议通过。当前 Story Spec 覆盖主方向，但安装任务 API 设计存在实现级歧义。\n\n\
        **主要问题**\n\n\
        - **High**：进度接口无法区分并发安装、重试安装、页面刷新后重连到哪一次任务。\n\n\
        ```json\n\
        {\"verdict\":\"needs_human\",\"summary\":\"安装任务 API 设计需修正。\"}\n\
        ```";

    let verdict = WorkspaceEngine::parse_review_verdict(output);

    assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
    assert_eq!(verdict.review_gate, ReviewGate::UserTriageRequired);
    assert_eq!(verdict.summary, "安装任务 API 设计需修正。");
    assert!(verdict.comments.contains("不建议通过"));
}

#[test]
fn parse_review_verdict_defaults_to_needs_human_when_contract_missing() {
    let output = "我无法确定是否通过，请人工确认。";

    let verdict = WorkspaceEngine::parse_review_verdict(output);

    assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
    assert_eq!(verdict.review_gate, ReviewGate::UserTriageRequired);
    assert_eq!(verdict.summary, "需要人工确认");
    assert_eq!(verdict.comments, output);
}

#[test]
fn parse_review_verdict_classifies_optional_findings_as_user_confirm_allowed() {
    let output = r#"整体可用，建议补充措辞。

```json
{
  "verdict": "revise",
  "summary": "有非阻塞建议",
  "findings": [
{
  "severity": "suggestion",
  "message": "建议补充边界说明",
  "evidence": "验收标准已经覆盖主路径",
  "impact": "不影响下一阶段执行",
  "required_action": "可在后续优化中补充"
}
  ]
}
```"#;

    let verdict = WorkspaceEngine::parse_review_verdict(output);

    assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
    assert_eq!(verdict.review_gate, ReviewGate::UserConfirmAllowed);
    assert_eq!(verdict.findings.len(), 1);
    assert_eq!(
        verdict.findings[0].severity,
        ReviewFindingSeverity::Suggestion
    );
}

#[test]
fn parse_review_verdict_classifies_strong_findings_as_requires_revision() {
    let output = r#"缺少 Work Item 可执行验证命令。

```json
{
  "verdict": "revise",
  "summary": "必须补充验证命令",
  "findings": [
{
  "severity": "must_fix",
  "message": "Work Item 没有验证命令",
  "evidence": "Artifact 未出现验证命令段落",
  "impact": "Coding Workspace 无法执行验收",
  "required_action": "补充明确验证命令"
}
  ]
}
```"#;

    let verdict = WorkspaceEngine::parse_review_verdict(output);

    assert_eq!(verdict.verdict, ReviewVerdictType::Revise);
    assert_eq!(verdict.review_gate, ReviewGate::RequiresRevision);
    assert_eq!(verdict.findings[0].severity, ReviewFindingSeverity::MustFix);
}

#[test]
fn parse_review_verdict_revise_without_findings_requires_user_triage() {
    let output = r#"建议修改一些描述。

```json
{"verdict":"revise","summary":"建议修改描述"}
```"#;

    let verdict = WorkspaceEngine::parse_review_verdict(output);

    assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
    assert_eq!(verdict.review_gate, ReviewGate::UserTriageRequired);
    assert!(verdict.findings.is_empty());
}
