#[tokio::test]
async fn single_review_round_strong_revise_still_pauses_for_decision() {
    let (_tmp, store) = setup();
    let (tx, _) = mpsc::channel(64);
    let mut session = make_session("sess_single_review_revise");
    session.review_rounds = 1;
    let mut engine = WorkspaceEngine::new(store, tx, session);

    engine
        .handle_user_message(
            "start".to_string(),
            Arc::new(FakeStreamingProvider),
            empty_provider_commands(),
        )
        .await;

    engine
        .drive_review_session(
            Arc::new(ReviewVerdictStreamingProvider {
                output: r#"需要移除非规范正文。

```json
{
  "verdict": "revise",
  "summary": "需要返修",
  "findings": [
{
  "severity": "strong_recommend_fix",
  "message": "存在非规范正文",
  "evidence": "Artifact 包含不符合模板的正文",
  "impact": "会影响下一阶段投影和审核",
  "required_action": "移除非规范正文"
}
  ]
}
```"#,
                provider_type: Arc::new(Mutex::new(None)),
                prompt: Arc::new(Mutex::new(None)),
            }),
            empty_provider_commands(),
        )
        .await;

    assert_eq!(engine.session().stage, WorkspaceStage::ReviewDecision);
    let active_node = engine
        .timeline_nodes
        .iter()
        .find(|node| Some(&node.node_id) == engine.active_node_id.as_ref())
        .expect("active review decision node");
    assert_eq!(active_node.node_type, TimelineNodeType::ReviewDecision);
    assert_eq!(active_node.status, TimelineNodeStatus::Paused);
}

#[tokio::test]
async fn review_decision_continue_after_strong_revise_runs_revision() {
    let (_tmp, store) = setup();
    let (tx, _) = mpsc::channel(64);
    let session = make_session("sess_review_revision");
    let mut engine = WorkspaceEngine::new(store, tx, session);

    engine
        .handle_user_message(
            "start".to_string(),
            Arc::new(FakeStreamingProvider),
            empty_provider_commands(),
        )
        .await;
    assert!(
        engine
            .session()
            .artifact
            .as_ref()
            .is_some_and(|artifact| artifact.markdown_or_empty().contains("# Story Spec"))
    );
    engine
        .handle_author_decision(AuthorDecision::Accept)
        .await
        .unwrap();
    assert_eq!(engine.session().stage, WorkspaceStage::CrossReview);

    engine
        .drive_review_session(
            Arc::new(ReviewVerdictStreamingProvider {
                output: r#"需要补充失败路径。

```json
{
  "verdict": "revise",
  "summary": "补充失败路径",
  "findings": [
{
  "severity": "must_fix",
  "message": "缺少失败路径",
  "evidence": "Artifact 未覆盖登录错误码",
  "impact": "下一阶段无法实现和验收失败路径",
  "required_action": "补充登录错误码失败路径"
}
  ]
}
```"#,
                provider_type: Arc::new(Mutex::new(None)),
                prompt: Arc::new(Mutex::new(None)),
            }),
            empty_provider_commands(),
        )
        .await;
    assert_eq!(engine.session().stage, WorkspaceStage::ReviewDecision);

    engine
        .handle_review_decision(
            "continue_with_context".to_string(),
            Some("补充登录错误码".to_string()),
        )
        .await
        .expect("decision should be accepted");
    assert_eq!(engine.session().stage, WorkspaceStage::Revision);

    let revision_provider_type = Arc::new(Mutex::new(None));
    let revision_prompt = Arc::new(Mutex::new(None));
    let revised_artifact = "# Story Spec\n\n\
        ## 范围\n\
        来源 source id: Issue issue_0001；覆盖补充失败路径后的版本。\n\n\
        ## 用户故事\n\
        作为用户，我希望失败路径有明确反馈。\n\n\
        ## 功能需求\n\
        - [REQ-001] 补充失败路径后的版本。\n\n\
        ## 成功标准\n\
        - [AC-001] 覆盖失败路径。\n\n\
        ## 待确认项\n\
        无。\n\n\
        ## 非功能需求\n\
        无。\n";
    engine
        .drive_revision_session(
            Arc::new(ReviewVerdictStreamingProvider {
                output: revised_artifact,
                provider_type: revision_provider_type.clone(),
                prompt: revision_prompt.clone(),
            }),
            empty_provider_commands(),
        )
        .await;

    assert_eq!(
        *revision_provider_type.lock().unwrap(),
        Some(ProviderType::ClaudeCode)
    );
    let prompt = revision_prompt
        .lock()
        .unwrap()
        .clone()
        .expect("revision prompt");
    assert!(prompt.contains("# Story Spec"));
    assert!(prompt.contains("需要补充失败路径"));
    assert!(prompt.contains("补充登录错误码"));
    assert!(prompt.contains("用户补充信息优先级高于 Reviewer 审核意见"));
    assert!(prompt.contains("如二者冲突，以用户补充信息为准"));
    assert!(prompt.contains("请根据以上审核意见修改产物"));
    assert_eq!(
        engine
            .session()
            .artifact
            .as_ref()
            .map(|payload| payload.markdown_or_empty()),
        Some(revised_artifact.trim())
    );
    engine
        .handle_author_decision(AuthorDecision::Accept)
        .await
        .unwrap();
    assert_eq!(engine.session().stage, WorkspaceStage::CrossReview);
    match engine.build_session_state() {
        WsOutMessage::SessionState {
            timeline_nodes,
            active_node_id,
            ..
        } => {
            assert!(timeline_nodes.iter().any(|node| {
                node.node_type == TimelineNodeType::Revision
                    && node.status == TimelineNodeStatus::Completed
                    && node.agent == Some(ProviderName::ClaudeCode)
            }));
            let active = timeline_nodes
                .iter()
                .find(|node| Some(&node.node_id) == active_node_id.as_ref())
                .expect("active review node");
            assert_eq!(active.node_type, TimelineNodeType::ReviewerRun);
            assert_eq!(active.round, Some(2));
        }
        _ => panic!("expected SessionState"),
    }
}

#[tokio::test]
async fn review_decision_with_context_requires_non_empty_context_for_all_workspace_types() {
    for workspace_type in [
        WorkspaceType::Story,
        WorkspaceType::Design,
        WorkspaceType::WorkItem,
    ] {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let mut session = make_session("sess_review_context_required");
        session.workspace_type = workspace_type.clone();
        session.stage = WorkspaceStage::ReviewDecision;
        let mut engine = WorkspaceEngine::new(store, tx, session);
        engine.latest_review_verdict = Some(ReviewVerdict {
            verdict: ReviewVerdictType::Revise,
            comments: "需要补充上下文后再返修。".to_string(),
            summary: "补充上下文".to_string(),
            findings: Vec::new(),
            review_gate: ReviewGate::RequiresRevision,
            work_item_plan_review: None,
        });

        let result = engine
            .handle_review_decision("continue_with_context".to_string(), Some("   ".to_string()))
            .await;

        assert_eq!(
            result,
            Err("continue_with_context requires non-empty extra_context".to_string())
        );
        assert_eq!(engine.session().stage, WorkspaceStage::ReviewDecision);
        assert!(
            !engine
                .timeline_nodes
                .iter()
                .any(|node| node.node_type == TimelineNodeType::Revision),
            "{workspace_type:?} should not create revision node without extra context"
        );
    }
}

#[tokio::test]
async fn review_decision_continue_with_work_item_plan_outline_candidate_restarts_outline() {
    let (tmp, checkpoint_store) = setup();
    let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let (tx, _rx) = mpsc::channel(64);
    let mut session = make_session("sess_wip_outline_review_fallback");
    session.workspace_type = WorkspaceType::WorkItemPlan;
    session.stage = WorkspaceStage::ReviewDecision;
    session.artifact = Some(ArtifactPayload::WorkItemPlanOutlineCandidate {
        outline_candidate: Box::new(WorkItemPlanOutlineCandidateDto {
            outline: WorkItemPlanOutline {
                id: "outline_001".to_string(),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                source_story_spec_ids: vec![],
                source_design_spec_ids: vec![],
                strategy_summary: "test".to_string(),
                work_item_outlines: vec![],
                dependency_graph: vec![],
                risks: vec![],
                handoff_strategy: "".to_string(),
                status: "draft".to_string(),
            },
            design_context_gaps: vec![],
            validator_findings: vec![],
            context_blockers: vec![],
            current_generation_round_id: None,
            selected_generation_mode: None,
        }),
    });
    let mut engine =
        WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store, tx, session);
    let node_id = "node_outline_confirm_001".to_string();
    engine.timeline_nodes.push(TimelineNode {
        node_id: node_id.clone(),
        node_type: TimelineNodeType::WorkItemPlanOutlineConfirm,
        agent: Some(ProviderName::ClaudeCode),
        stage: WsWorkspaceStage::AuthorConfirm,
        round: Some(1),
        status: TimelineNodeStatus::Paused,
        title: "WorkItemPlan Outline Confirm".to_string(),
        summary: None,
        started_at: chrono::Utc::now().to_rfc3339(),
        completed_at: None,
        duration_ms: None,
        artifact_ref: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::ClaudeCode,
            reviewer: Some(ProviderName::Codex),
            review_rounds: 1,
        },
        retry: None,
    });
    engine.active_node_id = Some(node_id);
    engine.latest_review_verdict = Some(ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: "revise".to_string(),
        summary: "revise".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::UserConfirmAllowed,
        work_item_plan_review: None,
    });

    let outcome = engine
        .handle_review_decision("continue".to_string(), None)
        .await
        .expect("decision should be accepted");

    assert_eq!(
        outcome,
        ReviewDecisionOutcome::StartWorkItemPlanOutlineRevision {
            feedback: Some("Reviewer 摘要: revise\n\nReviewer 审核意见:\nrevise".to_string()),
        }
    );
    assert_eq!(engine.session().stage, WorkspaceStage::Running);
    assert!(
        !engine
            .timeline_nodes
            .iter()
            .any(|node| node.node_type == TimelineNodeType::Revision),
        "should not create full revision node for outline candidate"
    );
}

#[tokio::test]
async fn revision_input_uses_persisted_codex_author_session_when_engine_session_is_stale() {
    let (tmp, checkpoint_store) = setup();
    let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let (tx, _) = mpsc::channel(64);
    let session_record = lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "story_spec_0001".to_string(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .unwrap();
    lifecycle_store
        .replace_workspace_provider_conversations(
            &session_record.id,
            vec![ProviderConversationRef {
                role: ProviderConversationRole::Author,
                provider: ProviderName::Codex,
                provider_session_id: "codex-author-session-1".to_string(),
                updated_at: chrono::Utc::now().to_rfc3339(),
                last_node_id: Some("timeline_node_002".to_string()),
            }],
        )
        .unwrap();

    let mut session = WorkspaceSession::from_record(session_record);
    session.stage = WorkspaceStage::Revision;
    session.artifact = Some(artifact_payload(
        "# Story Spec\n\n## 范围\n来源 source id: Issue issue_0001；初版。\n\n## 用户故事\n作为用户，我希望能力可用。\n\n## 功能需求\n- [REQ-001] 初版。\n\n## 成功标准\n- [AC-001] 初版可验收。\n\n## 待确认项\n无。\n\n## 非功能需求\n无。\n",
    ));
    session.messages.push(SessionMessage {
        id: "msg_001".to_string(),
        role: "system".to_string(),
        content: "很长的系统上下文，返修续接时不应重复发送。".to_string(),
        checkpoint_id: None,
        created_at: chrono::Utc::now().to_rfc3339(),
    });
    let mut engine =
        WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store, tx, session);
    engine.latest_review_verdict = Some(ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: "需要补充 reviewer 指出的 API 字段。".to_string(),
        summary: "补 API 字段".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
    });

    let input = engine.build_revision_input().expect("revision input");

    assert_eq!(
        input.resume_provider_session_id.as_deref(),
        Some("codex-author-session-1")
    );
    assert!(input.prompt.contains("需要补充 reviewer 指出的 API 字段。"));
    assert!(input.prompt.contains("输出完整更新后的 artifact markdown"));
    assert!(!input.prompt.contains("会话上下文:"));
    assert!(!input.prompt.contains("[system]:"));
    assert!(!input.prompt.contains("上一版 Artifact"));
    assert!(!input.prompt.contains("# Story Spec"));
}

#[tokio::test]
async fn revision_with_existing_author_provider_session_uses_delta_prompt() {
    let (_tmp, store) = setup();
    let (tx, _) = mpsc::channel(64);
    let mut session = make_session("sess_revision_delta_prompt");
    session.stage = WorkspaceStage::Revision;
    session.artifact = Some(artifact_payload(
        "# Story Spec\n\n## 功能需求\n- [REQ-001] 初版。\n\n## 成功标准\n- [AC-001] 初版可验收。\n",
    ));
    session.messages.push(SessionMessage {
        id: "msg_001".to_string(),
        role: "system".to_string(),
        content: "很长的系统上下文，返修续接时不应重复发送。".to_string(),
        checkpoint_id: None,
        created_at: chrono::Utc::now().to_rfc3339(),
    });
    session.messages.push(SessionMessage {
        id: "msg_002".to_string(),
        role: "assistant".to_string(),
        content: session
            .artifact
            .clone()
            .unwrap()
            .into_markdown()
            .expect("artifact"),
        checkpoint_id: None,
        created_at: chrono::Utc::now().to_rfc3339(),
    });
    session
        .provider_conversations
        .push(ProviderConversationRef {
            role: ProviderConversationRole::Author,
            provider: ProviderName::ClaudeCode,
            provider_session_id: "provider-author-session-1".to_string(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            last_node_id: Some("timeline_node_002".to_string()),
        });
    let mut engine = WorkspaceEngine::new(store, tx, session);
    engine.latest_review_verdict = Some(ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: "需要补充失败路径。".to_string(),
        summary: "补充失败路径".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
    });
    engine.pending_revision_context = Some("补充登录错误码".to_string());
    let captured_input = Arc::new(Mutex::new(None));

    engine
        .drive_revision_session(
            Arc::new(RevisionInputRecordingProvider {
                input: captured_input.clone(),
                output: "# Story Spec\n\n\
                    ## 范围\n来源 source id: Issue issue_0001；补充失败路径。\n\n\
                    ## 用户故事\n作为用户，我希望失败路径有明确反馈。\n\n\
                    ## 功能需求\n- [REQ-001] 补充失败路径。\n\n\
                    ## 成功标准\n- [AC-001] 覆盖失败路径。\n\n\
                    ## 待确认项\n无。\n\n\
                    ## 非功能需求\n无。\n",
            }),
            empty_provider_commands(),
        )
        .await;

    let input = captured_input
        .lock()
        .unwrap()
        .clone()
        .expect("revision provider input");
    assert_eq!(
        input.resume_provider_session_id.as_deref(),
        Some("provider-author-session-1")
    );
    assert!(input.prompt.contains("需要补充失败路径。"));
    assert!(input.prompt.contains("补充登录错误码"));
    assert!(
        input
            .prompt
            .contains("用户补充信息优先级高于 Reviewer 审核意见")
    );
    assert!(input.prompt.contains("如二者冲突，以用户补充信息为准"));
    assert!(input.prompt.contains("输出完整更新后的 artifact markdown"));
    assert!(!input.prompt.contains("会话上下文:"));
    assert!(!input.prompt.contains("[system]:"));
    assert!(!input.prompt.contains("上一版 Artifact"));
    assert!(!input.prompt.contains("# Story Spec"));
}

#[tokio::test]
async fn revision_codex_resume_stall_retries_fresh_full_prompt_for_all_workspace_types() {
    for (workspace_type, artifact, output) in [
        (
            WorkspaceType::Story,
            "# Story Spec\n\n## 范围\n来源 source id: Issue issue_0001；初版。\n\n## 用户故事\n作为用户，我希望能力可用。\n\n## 功能需求\n- [REQ-001] 初版。\n\n## 成功标准\n- [AC-001] 初版可验收。\n\n## 待确认项\n无。\n\n## 非功能需求\n无。\n",
            "# Story Spec\n\n## 范围\n来源 source id: Issue issue_0001；fresh 返修版本。\n\n## 用户故事\n作为用户，我希望能力可用。\n\n## 功能需求\n- [REQ-001] fresh 返修版本。\n\n## 成功标准\n- [AC-001] fresh 返修可验收。\n\n## 待确认项\n无。\n\n## 非功能需求\n无。\n",
        ),
        (
            WorkspaceType::Design,
            "# Design Spec\n\n## 设计范围\n初版。\n\n## 设计决策\n- [DEC-001] 初版。\n\n## 公共组件\n- [CMP-001] 初版组件。\n\n## API 契约\n- [API-001] 初版接口。\n\n## 数据模型\n- 沿用现有模型。\n\n## 风险\n无。\n\n## 追踪关系\n- source ids: Story Spec story_spec_0001, Issue issue_0001。\n- [DEC-001] -> [REQ-001]\n",
            "# Design Spec\n\n## 设计范围\nfresh 返修版本。\n\n## 设计决策\n- [DEC-001] fresh 返修版本。\n\n## 公共组件\n- [CMP-001] fresh 组件。\n\n## API 契约\n- [API-001] fresh 返修接口。\n\n## 数据模型\n- 沿用现有模型。\n\n## 风险\n无。\n\n## 追踪关系\n- source ids: Story Spec story_spec_0001, Issue issue_0001。\n- [DEC-001] -> [REQ-001]\n",
        ),
        (
            WorkspaceType::WorkItem,
            "# Work Item\n\n## 目标\n初版任务。\n\n## 范围\n仅当前任务。\n\n## 实现步骤\n- 实现当前任务。\n\n## 依赖\n无。\n\n## 验证命令\n- cargo test --locked\n\n## 风险\n无。\n\n## 追踪关系\n- source ids: Story Spec story_spec_0001, Design Spec design_spec_0001。\n- [REQ-001]\n",
            "# Work Item\n\n## 目标\nfresh 返修任务。\n\n## 范围\n仅当前任务。\n\n## 实现步骤\n- 实现当前任务。\n\n## 依赖\n无。\n\n## 验证命令\n- cargo test --locked\n\n## 风险\n无。\n\n## 追踪关系\n- source ids: Story Spec story_spec_0001, Design Spec design_spec_0001。\n- [REQ-001]\n",
        ),
    ] {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let mut session = make_session(&format!("sess_revision_resume_stall_{workspace_type:?}"));
        session.workspace_type = workspace_type.clone();
        session.stage = WorkspaceStage::ReviewDecision;
        session.artifact = Some(artifact_payload(artifact));
        session.author_provider = ProviderName::Codex;
        session.messages.push(SessionMessage {
            id: "msg_001".to_string(),
            role: "assistant".to_string(),
            content: artifact.to_string(),
            checkpoint_id: None,
            created_at: chrono::Utc::now().to_rfc3339(),
        });
        session
            .provider_conversations
            .push(ProviderConversationRef {
                role: ProviderConversationRole::Author,
                provider: ProviderName::Codex,
                provider_session_id: "codex-stale-ephemeral-thread".to_string(),
                updated_at: chrono::Utc::now().to_rfc3339(),
                last_node_id: Some("timeline_node_002".to_string()),
            });
        let mut engine = WorkspaceEngine::new(store, tx, session);
        engine.latest_review_verdict = Some(ReviewVerdict {
            verdict: ReviewVerdictType::Revise,
            comments: "需要补充失败路径。".to_string(),
            summary: "补充失败路径".to_string(),
            findings: Vec::new(),
            review_gate: ReviewGate::RequiresRevision,
            work_item_plan_review: None,
        });
        engine
            .handle_review_decision(
                "continue_with_context".to_string(),
                Some("补充旧 session 不可恢复时的处理。".to_string()),
            )
            .await
            .expect("review decision should enter revision");

        let inputs = Arc::new(Mutex::new(Vec::new()));
        engine
            .drive_revision_session(
                Arc::new(RevisionResumeStallThenSuccessProvider {
                    inputs: inputs.clone(),
                    calls: Arc::new(Mutex::new(0)),
                    output,
                }),
                empty_provider_commands(),
            )
            .await;

        let inputs = inputs.lock().unwrap().clone();
        assert_eq!(inputs.len(), 2, "{workspace_type:?} should retry once");
        assert_eq!(
            inputs[0].resume_provider_session_id.as_deref(),
            Some("codex-stale-ephemeral-thread")
        );
        assert!(
            !inputs[0].prompt.contains("上一版 Artifact"),
            "{workspace_type:?} first resume attempt should use delta prompt"
        );
        assert_eq!(inputs[1].resume_provider_session_id, None);
        assert!(
            inputs[1].prompt.contains("上一版 Artifact"),
            "{workspace_type:?} fresh retry should use full prompt"
        );
        assert!(
            inputs[1].prompt.contains(artifact.trim()),
            "{workspace_type:?} fresh retry should include prior artifact"
        );
        assert_eq!(
            engine
                .session()
                .artifact
                .as_ref()
                .map(|payload| payload.markdown_or_empty()),
            Some(output.trim())
        );
        assert_eq!(
            engine
                .provider_resume_session_id(ProviderConversationRole::Author, &ProviderName::Codex,)
                .as_deref(),
            Some("codex-fresh-thread")
        );
    }
}

#[test]
fn revision_input_reminds_design_author_to_return_artifact_fenced_block() {
    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut session = make_session("sess_design_revision_prompt_fence_contract");
    session.workspace_type = WorkspaceType::Design;
    session.stage = WorkspaceStage::Revision;
    session.artifact = Some(artifact_payload(
        "# 底层依赖安装任务 Design Spec\n\n\
         ## 设计范围\n\n\
         - 覆盖依赖安装任务。\n\n\
         ## API 契约\n\n\
         ```json\n\
         {\"task_id\":\"install_001\"}\n\
         ```\n",
    ));
    let checkpoint_tmp = TempDir::new().unwrap();
    let mut engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
        event_tx,
        session,
    );
    engine.latest_review_verdict = Some(ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: "需要补齐追踪关系。".to_string(),
        summary: "补齐追踪关系".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
    });

    let input = engine.build_revision_input().expect("revision input");

    assert!(
        input
            .prompt
            .contains("原始返回必须使用完整 artifact fenced block"),
        "revision author prompt should require the raw artifact fence: {}",
        input.prompt
    );
    assert!(
        input
            .prompt
            .contains("正文内部包含 ``` 代码块时，外层使用四反引号 ````artifact"),
        "revision author prompt should explain four-backtick outer fence: {}",
        input.prompt
    );
    assert!(
        input
            .prompt
            .contains("上一版 Artifact 是 daemon 已提取的 markdown"),
        "revision author prompt should explain why prior artifact has no fence: {}",
        input.prompt
    );
}

#[tokio::test]
async fn revision_delta_prompt_includes_legacy_context_note() {
    let (_tmp, store) = setup();
    let (tx, _) = mpsc::channel(64);
    let mut session = make_session("sess_revision_delta_legacy_context_note");
    session.stage = WorkspaceStage::Revision;
    session.artifact = Some(artifact_payload(
        "# Story Spec\n\n## 功能需求\n- [REQ-001] 初版。\n\n## 成功标准\n- [AC-001] 初版可验收。\n",
    ));
    session
        .provider_conversations
        .push(ProviderConversationRef {
            role: ProviderConversationRole::Author,
            provider: ProviderName::ClaudeCode,
            provider_session_id: "provider-author-session-1".to_string(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            last_node_id: Some("timeline_node_002".to_string()),
        });
    let mut engine = WorkspaceEngine::new(store, tx, session);
    engine.latest_review_verdict = Some(ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: "需要补充验收值。".to_string(),
        summary: "补充验收值".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
    });
    engine
        .append_completed_timeline_event(
            TimelineNodeType::ContextNote,
            WorkspaceStage::PrepareContext,
            "上下文补充".to_string(),
            Some("旧现场补充：必须覆盖 n=10 -> 89。".to_string()),
            TimelineNodeStatus::Completed,
            false,
        )
        .await;

    let input = engine.build_revision_input().expect("revision input");

    assert_eq!(
        input.resume_provider_session_id.as_deref(),
        Some("provider-author-session-1")
    );
    assert!(
        input.prompt.contains("旧现场补充：必须覆盖 n=10 -> 89。"),
        "revision author prompt should include legacy context note, got: {}",
        input.prompt
    );
}

struct RevisionInputRecordingProvider {
    input: Arc<Mutex<Option<StreamingProviderInput>>>,
    output: &'static str,
}

struct RevisionResumeStallThenSuccessProvider {
    inputs: Arc<Mutex<Vec<StreamingProviderInput>>>,
    calls: Arc<Mutex<u32>>,
    output: &'static str,
}
