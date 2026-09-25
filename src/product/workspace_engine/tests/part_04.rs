// 退役留档（T5/REQ-RET-02）：`single_review_round_strong_revise_still_pauses_for_author_decision` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`strong_revise_then_author_feedback_runs_revision` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`review_decision_with_context_requires_non_empty_context_for_all_workspace_types` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`review_decision_continue_with_work_item_plan_outline_candidate_restarts_outline` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

#[tokio::test]
async fn revision_input_uses_persisted_codex_author_session_when_engine_session_is_stale() {
    let (tmp, checkpoint_store) = setup();
    let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let (tx, _) = mpsc::channel(64);
    let session_record = lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput { project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        entity_id: "story_spec_0001".to_string(),
        workspace_type: WorkspaceType::Story,
        author_provider: ProviderName::Codex,
        reviewer_provider: ProviderName::ClaudeCode,
        review_rounds: 1,
        superpowers_enabled: true, openspec_enabled: true, work_item_plan_options: None, })
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
        structured_output_diagnostic: None,
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
    // F-60 P0：delta 出口装配含骨架示例（`# Story Spec 标题`），旧产物 H1
    //（`# Story Spec` + 换行）仍不得回放。
    assert!(!input.prompt.contains("# Story Spec\n"));
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
        structured_output_diagnostic: None,
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
    // F-60 P0：delta 出口装配含骨架示例（`# Story Spec 标题`），旧产物 H1
    //（`# Story Spec` + 换行）仍不得回放。
    assert!(!input.prompt.contains("# Story Spec\n"));
    assert!(!input.prompt.contains("上一版 Artifact"));
}

#[tokio::test]
async fn revision_prompt_requires_structured_interaction_decisions_in_artifact() {
    let (_tmp, store) = setup();
    let (tx, _) = mpsc::channel(64);
    let mut session = make_session("sess_revision_structured_decisions");
    session.stage = WorkspaceStage::Revision;
    session.workspace_type = WorkspaceType::Story;
    session.artifact = Some(artifact_payload(
        "# Story Spec\n\n## 功能需求\n- [REQ-001] 初版。\n\n## 成功标准\n- [AC-001] 初版可验收。\n",
    ));
    session.messages.push(SessionMessage {
        id: "msg_001".to_string(),
        role: "system".to_string(),
        content: "结构化交互审计记录（daemon 捕获）\n- choice_id: choice_install_policy\n- source: ask_user_question\n- answers:\n  - question_id: q1\n    question: Claude Code 是否必装？\n    selected: mandatory_blocking = 必装且阻断\n".to_string(),
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
        comments: "需要把结构化交互确认的安装策略写入 Story Spec。".to_string(),
        summary: "补充交互决策映射".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    });

    let input = engine.build_revision_input().expect("revision input");

    // F-60 P0 分层：resume delta 轮出口为短引用——决策归档以指针形式在场
    //（author-decision-*），完整决策契约见会话开头合同。
    assert!(input.prompt.contains("决策归档"));
    assert!(input.prompt.contains("author-decision"));
    assert!(input.prompt.contains("[REQ-"));
    assert!(input.prompt.contains("[AC-"));
    assert!(input.prompt.contains("待确认项"));
}

// 退役留档（T5/REQ-RET-02）：`revision_codex_resume_stall_retries_fresh_full_prompt_for_all_workspace_types` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

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
        structured_output_diagnostic: None,
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
        structured_output_diagnostic: None,
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
