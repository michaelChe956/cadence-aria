#[test]
fn provider_resume_session_id_is_isolated_by_role_and_provider() {
    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut session = make_session("sess_role_isolation");
    session.author_provider = ProviderName::ClaudeCode;
    session.reviewer_provider = Some(ProviderName::ClaudeCode);
    session.provider_conversations = vec![ProviderConversationRef {
        role: ProviderConversationRole::Author,
        provider: ProviderName::ClaudeCode,
        provider_session_id: "author-session".to_string(),
        updated_at: "2026-06-01T00:00:00Z".to_string(),
        last_node_id: Some("node-author".to_string()),
    }];
    let checkpoint_tmp = TempDir::new().unwrap();
    let engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
        event_tx,
        session,
    );

    assert_eq!(
        engine.provider_resume_session_id(
            ProviderConversationRole::Author,
            &ProviderName::ClaudeCode
        ),
        Some("author-session".to_string())
    );
    assert_eq!(
        engine.provider_resume_session_id(
            ProviderConversationRole::Reviewer,
            &ProviderName::ClaudeCode
        ),
        None
    );
    assert_eq!(
        engine.provider_resume_session_id(ProviderConversationRole::Author, &ProviderName::Codex),
        None
    );
}

#[test]
fn design_artifact_gate_accepts_numbered_canonical_headings() {
    let content = r#"# Provider 依赖自检 Design Spec

## 1. 设计范围

本设计覆盖 provider 依赖自检与安装。

## 2. 设计决策

- [DEC-001] 新建 ProviderCatalog。

## 3. 公共组件

- [CMP-001] ProviderCatalog。

## 4. API 契约

- [API-001] ProviderCatalog::probe。

## 5. 数据模型

- ProviderCapability。

## 6. 风险

无。

## 7. 追踪关系

- source ids: Story Spec story_spec_0001, Issue issue_0001
- [DEC-001] -> [REQ-001]
"#;

    assert!(content_has_complete_workspace_artifact(
        content,
        &WorkspaceType::Design
    ));
}

#[test]
fn design_artifact_gate_rejects_legacy_key_decision_heading() {
    let content = r#"# Provider 依赖自检 Design Spec

## 1. 设计范围

本设计覆盖 provider 依赖自检与安装。

## 关键决策

- [DEC-001] 新建 ProviderCatalog。

## 组件 / API / 数据模型

- [CMP-001] ProviderCatalog。
"#;

    assert!(!content_has_complete_workspace_artifact(
        content,
        &WorkspaceType::Design
    ));
}

#[test]
fn review_input_does_not_resume_prior_reviewer_provider_session() {
    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut session = make_session("sess_review_no_resume");
    session.reviewer_provider = Some(ProviderName::Codex);
    session.artifact = Some(artifact_payload(
        "# Story Spec\n\n## 功能需求\n- [REQ-001] Draft.\n",
    ));
    session.provider_conversations = vec![ProviderConversationRef {
        role: ProviderConversationRole::Reviewer,
        provider: ProviderName::Codex,
        provider_session_id: "codex-review-thread-1".to_string(),
        updated_at: "2026-06-01T00:00:00Z".to_string(),
        last_node_id: Some("timeline_node_003".to_string()),
    }];
    let checkpoint_tmp = TempDir::new().unwrap();
    let engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
        event_tx,
        session,
    );

    let input = engine.build_review_input().expect("review input");

    assert_eq!(input.resume_provider_session_id, None);
    assert!(input.prompt.contains("当前 Artifact"));
}

#[tokio::test]
async fn build_session_state_inlines_sanitized_work_item_plan_run_details() {
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
        node_id: "outline-node".to_string(),
        node_type: TimelineNodeType::WorkItemPlanOutlineRun,
        agent: Some(ProviderName::ClaudeCode),
        stage: WsWorkspaceStage::Running,
        round: None,
        status: TimelineNodeStatus::Completed,
        title: "生成 Work Item Plan Outline".to_string(),
        summary: None,
        started_at: "2026-05-20T14:30:00Z".to_string(),
        completed_at: Some("2026-05-20T14:35:00Z".to_string()),
        duration_ms: Some(300000),
        artifact_ref: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::ClaudeCode,
            reviewer: None,
            review_rounds: 0,
            permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
        },
        retry: None,
    });
    let huge_prompt = "P".repeat(3000);
    let huge_output = "O".repeat(3000);
    lifecycle_store
        .save_node_detail(
            &session_id,
            "outline-node",
            &NodeDetail {
                node_id: "outline-node".to_string(),
                session_id: session_id.clone(),
                node_type: TimelineNodeType::WorkItemPlanOutlineRun,
                status: TimelineNodeStatus::Completed,
                agent_role: Some(AgentRole::Author),
                provider: Some(ProviderSnapshot {
                    name: "claude_code".to_string(),
                    model: "claude-opus-4-7".to_string(),
                }),
                prompt: Some(huge_prompt.clone()),
                messages: vec![serde_json::json!({"content": "hidden message"})],
                streaming_content: "Fake Work Item Plan streaming draft".to_string(),
                execution_events: vec![serde_json::json!({
                    "event_id": "provider_output",
                    "kind": "output",
                    "status": "completed",
                    "title": "Provider output",
                    "output": huge_output,
                })],
                permission_events: vec![PermissionEvent {
                    request_id: "perm-1".to_string(),
                    request: serde_json::json!({"tool": "shell"}),
                    response: Some(serde_json::json!({"approved": true})),
                    ts: "2026-05-20T14:31:00Z".to_string(),
                }],
                verdict: None,
                artifact_ref: None,
                is_revision: false,
                revision_feedback: None,
                base_artifact_ref: None,
                started_at: "2026-05-20T14:30:00Z".to_string(),
                ended_at: Some("2026-05-20T14:35:00Z".to_string()),
            },
        )
        .unwrap();

    let state = engine.build_session_state();
    let serialized = serde_json::to_string(&state).unwrap();
    match state {
        WsOutMessage::SessionState {
            timeline_node_details,
            timeline_node_summaries,
            ..
        } => {
            let detail = timeline_node_details
                .get("outline-node")
                .expect("outline run detail should be inlined");
            assert!(detail.prompt.is_none());
            assert!(detail.messages.is_empty());
            assert_eq!(
                detail.streaming_content,
                "Fake Work Item Plan streaming draft"
            );
            assert!(detail.permission_events.is_empty());
            assert_eq!(detail.execution_events.len(), 1);
            assert!(
                detail.execution_events[0]
                    .get("output")
                    .is_some_and(serde_json::Value::is_null)
            );
            assert!(timeline_node_summaries.is_empty());
        }
        _ => panic!("expected SessionState"),
    }
    assert!(!serialized.contains(&huge_prompt));
    assert!(!serialized.contains(&huge_output));
    assert!(!serialized.contains("hidden message"));
}
