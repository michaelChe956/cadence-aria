#[tokio::test]
async fn strong_review_findings_enter_review_decision_for_all_workspace_types() {
    for workspace_type in [
        WorkspaceType::Story,
        WorkspaceType::Design,
        WorkspaceType::WorkItem,
    ] {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let mut session = make_session(&format!("sess_strong_review_{workspace_type:?}"));
        session.workspace_type = workspace_type.clone();
        session.review_rounds = 2;
        session.artifact = Some(artifact_payload("# Artifact\n\n缺少验收标准"));
        let mut engine = WorkspaceEngine::new(store, tx, session);
        engine.start_review_or_skip().await;

        engine
            .drive_review_session(
                Arc::new(ReviewVerdictStreamingProvider {
                    output: r#"必须补充验收标准。

```json
{
  "verdict": "revise",
  "summary": "必须补充验收标准",
  "findings": [
{
  "severity": "strong_recommend_fix",
  "message": "验收标准不足",
  "evidence": "Artifact 未列出可测试验收值",
  "impact": "下一阶段无法判断实现是否完成",
  "required_action": "补充明确验收标准"
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
        assert!(
            engine
                .timeline_nodes
                .iter()
                .any(|node| node.node_type == TimelineNodeType::ReviewDecision),
            "{workspace_type:?} should require revision for strong findings"
        );
    }
}

#[tokio::test]
async fn revise_without_findings_enters_user_triage_for_all_workspace_types() {
    for workspace_type in [
        WorkspaceType::Story,
        WorkspaceType::Design,
        WorkspaceType::WorkItem,
    ] {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let mut session = make_session(&format!("sess_triage_review_{workspace_type:?}"));
        session.workspace_type = workspace_type.clone();
        session.review_rounds = 2;
        session.artifact = Some(artifact_payload("# Artifact\n\n需要人工裁决的版本"));
        let mut engine = WorkspaceEngine::new(store, tx, session);
        engine.start_review_or_skip().await;

        engine
            .drive_review_session(
                Arc::new(ReviewVerdictStreamingProvider {
                    output: r#"Reviewer 明确要求返修，但未输出结构化 finding。

```json
{
  "verdict": "revise",
  "summary": "返修意图需要人工判断"
}
```"#,
                    provider_type: Arc::new(Mutex::new(None)),
                    prompt: Arc::new(Mutex::new(None)),
                }),
                empty_provider_commands(),
            )
            .await;

        assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
        assert_eq!(
            engine
                .latest_review_verdict
                .as_ref()
                .expect("latest review verdict")
                .review_gate,
            ReviewGate::UserTriageRequired
        );
        assert!(
            engine
                .timeline_nodes
                .iter()
                .any(|node| node.node_type == TimelineNodeType::HumanConfirm),
            "{workspace_type:?} should create human_confirm node for user triage"
        );
        assert!(
            !engine
                .timeline_nodes
                .iter()
                .any(|node| node.node_type == TimelineNodeType::ReviewDecision),
            "{workspace_type:?} should not auto-revise unstructured review intent"
        );
    }
}

#[tokio::test]
async fn malformed_findings_enter_user_triage_for_all_workspace_types() {
    for workspace_type in [
        WorkspaceType::Story,
        WorkspaceType::Design,
        WorkspaceType::WorkItem,
    ] {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let mut session = make_session(&format!("sess_malformed_review_{workspace_type:?}"));
        session.workspace_type = workspace_type.clone();
        session.review_rounds = 2;
        session.artifact = Some(artifact_payload("# Artifact\n\n需要人工裁决的版本"));
        let mut engine = WorkspaceEngine::new(store, tx, session);
        engine.start_review_or_skip().await;

        engine
            .drive_review_session(
                Arc::new(ReviewVerdictStreamingProvider {
                    output: r#"Reviewer 明确要求返修，但 findings 结构错误。

```json
{
  "verdict": "revise",
  "summary": "findings 无法可靠解析",
  "findings": [{"severity": "must_fix", "message": 42}]
}
```"#,
                    provider_type: Arc::new(Mutex::new(None)),
                    prompt: Arc::new(Mutex::new(None)),
                }),
                empty_provider_commands(),
            )
            .await;

        assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
        assert_eq!(
            engine
                .latest_review_verdict
                .as_ref()
                .expect("latest review verdict")
                .review_gate,
            ReviewGate::UserTriageRequired
        );
    }
}

#[test]
fn review_prompt_limits_revise_to_strong_findings() {
    let (_tmp, store) = setup();
    let (tx, _) = mpsc::channel(8);
    let mut session = make_session("sess_review_prompt_gate");
    session.artifact = Some(artifact_payload("# Story Spec\n\n可用版本"));
    let engine = WorkspaceEngine::new(store, tx, session);

    let input = engine.build_review_input().expect("review input");

    assert!(
        input
            .prompt
            .contains("blocking|must_fix|strong_recommend_fix")
    );
    assert!(input.prompt.contains("suggestion|minor|optional"));
    assert!(
        input
            .prompt
            .contains("没有强返修 finding 时，必须允许用户确认当前版本")
    );
    assert!(
        !input
            .prompt
            .contains("High/Medium 问题、建议改动或可执行返修项，必须使用 `revise`")
    );
    assert!(
        input
            .prompt
            .contains("如果输出 `verdict=revise`，必须给出至少一个结构化 finding")
    );
}

#[test]
fn detect_author_choice_request_accepts_markdown_bold_bulleted_options() {
    let output = "感谢提供项目上下文。\n\n\
        在生成 Story Spec 之前，我有几个问题需要确认：\n\n\
        **问题 1：弹窗触发时机**\n\n\
        根据 Issue 描述，弹窗是在\"启动 aria 后\"触发。请问这里的\"启动 aria\"具体指什么时机？\n\n\
        - **A)** 用户运行 `aria` 命令启动 daemon 时（Rust 后端启动时）\n\
        - **B)** 用户打开 Web 工作台页面时（前端首次加载时）\n\
        - **C)** 两者都需要（后端启动时检测，前端展示弹窗）\n";

    let (prompt, options) = detect_author_choice_request(output, &WorkspaceType::Story)
        .expect("markdown bold bulleted options should become a choice request");

    assert!(prompt.contains("弹窗触发时机"));
    assert_eq!(options.len(), 3);
    assert_eq!(options[0].id, "A");
    assert!(options[0].label.contains("用户运行 `aria`"));
    assert_eq!(options[1].id, "B");
    assert_eq!(options[2].id, "C");
}

#[test]
fn detect_author_choice_request_uses_nearest_question_for_codex_numbered_options() {
    let output = "我会先读取本仓库规则和必须使用的技能说明，然后根据未决点用结构化提问确认范围，再产出候选 Story Spec。\
        规则侧已经明确：这次最终只输出候选 Markdown，不落盘、不改 OpenSpec。\
        结构化提问工具当前不可用，我先用文本方式提问：\n\n\
        首次启动检测到缺失 Claude Code/Codex 时，Aria 应采用哪种安装策略？\n\n\
        1. `确认后安装`：弹窗展示将执行的 npm 安装命令，用户点击安装后才执行。\n\
        2. `自动静默安装`：检测缺失后直接运行 npm 安装。\n\
        3. `只检查不安装`：只展示缺失与命令，由用户自行安装。\n\n\
        我建议选 `确认后安装`，因为它满足“自动检查与自动安装”。";

    let (prompt, options) = detect_author_choice_request(output, &WorkspaceType::Story)
        .expect("Codex numbered text question should become a choice request");

    assert_eq!(
        prompt,
        "首次启动检测到缺失 Claude Code/Codex 时，Aria 应采用哪种安装策略？"
    );
    assert!(!prompt.contains("我会先读取本仓库规则"));
    assert!(!prompt.contains("结构化提问工具当前不可用"));
    assert_eq!(options.len(), 3);
    assert_eq!(options[0].id, "1");
    assert!(options[0].label.contains("确认后安装"));
    assert_eq!(options[1].id, "2");
    assert_eq!(options[2].id, "3");
}

struct ReviewVerdictStreamingProvider {
    output: &'static str,
    provider_type: Arc<Mutex<Option<ProviderType>>>,
    prompt: Arc<Mutex<Option<String>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ReviewVerdictStreamingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        *self.provider_type.lock().unwrap() = Some(input.provider_type.clone());
        *self.prompt.lock().unwrap() = Some(input.prompt.clone());
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        let output = self.output.to_string();
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: output.clone(),
                })
                .await;
            let _ = event_tx
                .send(ProviderEvent::Completed {
                    full_output: output,
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
async fn drive_review_session_pass_enters_human_confirm() {
    let (_tmp, store) = setup();
    let (tx, mut rx) = mpsc::channel(64);
    let session = make_session("sess_review_pass");
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
    assert_eq!(engine.session().stage, WorkspaceStage::CrossReview);

    let provider_type = Arc::new(Mutex::new(None));
    let prompt = Arc::new(Mutex::new(None));
    engine
        .drive_review_session(
            Arc::new(ReviewVerdictStreamingProvider {
                output: "审核通过。\n\n```json\n{\"verdict\":\"pass\",\"summary\":\"可以确认\"}\n```",
                provider_type: provider_type.clone(),
                prompt: prompt.clone(),
            }),
            empty_provider_commands(),
        )
        .await;

    assert_eq!(*provider_type.lock().unwrap(), Some(ProviderType::Codex));
    assert!(
        prompt
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .contains("# Story Spec")
    );
    assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
    match engine.build_session_state() {
        WsOutMessage::SessionState { timeline_nodes, .. } => {
            assert!(timeline_nodes.iter().any(|node| {
                node.node_type == TimelineNodeType::ReviewerRun
                    && node.status == TimelineNodeStatus::Completed
                    && node.summary.as_deref() == Some("可以确认")
            }));
        }
        _ => panic!("expected SessionState"),
    }

    let mut saw_review_complete = false;
    while let Ok(event) = rx.try_recv() {
        if let EngineEvent::ReviewComplete {
            verdict,
            summary,
            findings,
            review_gate,
            ..
        } = event
        {
            assert_eq!(verdict, ReviewVerdictType::Pass);
            assert_eq!(summary, "可以确认");
            assert!(findings.is_empty());
            assert_eq!(review_gate, ReviewGate::UserConfirmAllowed);
            saw_review_complete = true;
        }
    }
    assert!(saw_review_complete);
}

#[tokio::test]
async fn drive_review_session_strong_revise_pauses_for_decision() {
    let (_tmp, store) = setup();
    let (tx, _) = mpsc::channel(64);
    let session = make_session("sess_review_revise");
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
                output: r#"需要补充失败路径。

```json
{
  "verdict": "revise",
  "summary": "补充失败路径",
  "findings": [
{
  "severity": "must_fix",
  "message": "缺少失败路径",
  "evidence": "Artifact 未覆盖失败路径",
  "impact": "下一阶段无法验收异常流程",
  "required_action": "补充失败路径说明"
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
    match engine.build_session_state() {
        WsOutMessage::SessionState {
            timeline_nodes,
            active_node_id,
            ..
        } => {
            let active = timeline_nodes
                .iter()
                .find(|node| Some(&node.node_id) == active_node_id.as_ref())
                .expect("active review decision node");
            assert_eq!(active.node_type, TimelineNodeType::ReviewDecision);
            assert_eq!(active.status, TimelineNodeStatus::Paused);
        }
        _ => panic!("expected SessionState"),
    }
}
