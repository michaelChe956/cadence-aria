// spec-design-dialog-revision T5：Story/Design 强 revise 完成统一回 AuthorConfirm（报告进对话流），
// WorkItem 维持既有 ReviewDecision 路由（design.md「WorkItem 不受影响」）。
#[tokio::test]
async fn strong_review_findings_route_author_confirm_or_review_decision_for_all_workspace_types() {
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
        engine.start_review().await;

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
  "severity": "must_fix",
  "message": "验收标准不足",
  "evidence": "Artifact 未列出可测试验收值",
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

        match workspace_type {
            WorkspaceType::Story | WorkspaceType::Design => {
                assert_eq!(
                    engine.session().stage,
                    WorkspaceStage::AuthorConfirm,
                    "{workspace_type:?} strong revise 完成必须回 AuthorConfirm（报告进对话流）"
                );
                assert!(
                    engine
                        .timeline_nodes
                        .iter()
                        .any(|node| node.node_type == TimelineNodeType::AuthorConfirm),
                    "{workspace_type:?} should route review report back to author confirm"
                );
            }
            WorkspaceType::WorkItem => {
                assert_eq!(engine.session().stage, WorkspaceStage::ReviewDecision);
                assert!(
                    engine
                        .timeline_nodes
                        .iter()
                        .any(|node| node.node_type == TimelineNodeType::ReviewDecision),
                    "{workspace_type:?} should require revision for strong findings"
                );
            }
            other => panic!("unexpected workspace type {other:?}"),
        }
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
        engine.start_review().await;

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

        // review_gate 语义不变（fallback 判定仍为 UserTriageRequired）；仅路由终点按类型分流：
        // Story/Design → AuthorConfirm（报告进对话流），WorkItem → HumanConfirm（用户 triage）。
        assert_eq!(
            engine
                .latest_review_verdict
                .as_ref()
                .expect("latest review verdict")
                .review_gate,
            ReviewGate::UserTriageRequired
        );
        match workspace_type {
            WorkspaceType::Story | WorkspaceType::Design => {
                assert_eq!(
                    engine.session().stage,
                    WorkspaceStage::AuthorConfirm,
                    "{workspace_type:?} revise 无 findings 必须回 AuthorConfirm（报告进对话流）"
                );
            }
            WorkspaceType::WorkItem => {
                assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
                assert!(
                    engine
                        .timeline_nodes
                        .iter()
                        .any(|node| node.node_type == TimelineNodeType::HumanConfirm),
                    "{workspace_type:?} should create human_confirm node for user triage"
                );
            }
            other => panic!("unexpected workspace type {other:?}"),
        }
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
        engine.start_review().await;

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

        assert_eq!(
            engine
                .latest_review_verdict
                .as_ref()
                .expect("latest review verdict")
                .review_gate,
            ReviewGate::UserTriageRequired
        );
        match workspace_type {
            WorkspaceType::Story | WorkspaceType::Design => {
                assert_eq!(
                    engine.session().stage,
                    WorkspaceStage::AuthorConfirm,
                    "{workspace_type:?} malformed findings 必须回 AuthorConfirm（报告进对话流）"
                );
            }
            WorkspaceType::WorkItem => {
                assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
            }
            other => panic!("unexpected workspace type {other:?}"),
        }
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
            .contains("blocking|must_fix|suggestion")
    );
    assert!(input.prompt.contains("suggestion"));
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

#[test]
fn four_backtick_artifact_extracts_across_workspace_types_and_suppresses_story_design_fallback() {
    for (workspace_type, artifact) in [
        (
            WorkspaceType::Story,
            complete_story_artifact(
                "用户遇到失败时应该如何处理？",
                "失败路径有明确提示。",
            ),
        ),
        (
            WorkspaceType::Design,
            complete_design_artifact(
                "明确失败时应该如何处理？",
                "返回类型化失败原因。",
            ),
        ),
        (
            WorkspaceType::WorkItem,
            complete_work_item_artifact("实现失败时应该如何处理？"),
        ),
    ] {
        let output = format!(
            "基于 reviewer 意见完成两处返修：\n\n\
             1. 补齐失败处理。\n\
             2. 保留既有兼容约束。\n\n\
             更新后的完整 artifact：\n\n\
             ````artifact\n{artifact}````\n\n\
             返修完成。"
        );

        let selection = select_workspace_artifact(&output, workspace_type.clone());
        assert_eq!(
            selected_artifact_markdown(&selection).as_deref(),
            Some(artifact.trim()),
            "{workspace_type:?} 应唯一选中四反引号 artifact 正文"
        );
        assert!(
            content_has_complete_workspace_artifact(&output, &workspace_type),
            "{workspace_type:?} 四反引号 artifact 应通过完整性校验"
        );

        if matches!(workspace_type, WorkspaceType::Story | WorkspaceType::Design) {
            assert!(
                detect_author_choice_request(&output, &workspace_type).is_none(),
                "完整四反引号 artifact 不应被误判为 {workspace_type:?} 文本选择题"
            );
        }
    }
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
        let structured_output_contract = input.structured_output_contract;
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: output.clone(),
                })
                .await;
            let full_output = if let Some(contract) = structured_output_contract.as_ref() {
                let (comments, json) =
                    extract_structured_json(&output).expect("review fixture structured output");
                let mut json: serde_json::Value =
                    serde_json::from_str(&json).expect("review fixture JSON must be an object");
                json.as_object_mut()
                    .expect("review fixture JSON must be an object")
                    .insert("nonce".to_string(), serde_json::json!(contract.nonce));
                format!(
                    "{comments}\n<ARIA_STRUCTURED_OUTPUT nonce=\"{}\">{json}</ARIA_STRUCTURED_OUTPUT>",
                    contract.nonce
                )
            } else {
                output
            };
            let _ = event_tx
                .send(ProviderEvent::Completed(
                    crate::cross_cutting::streaming_provider::ProviderCompletion::from_output(
                        full_output,
                        structured_output_contract.as_ref(),
                        None,
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

// 退役留档（T5/REQ-RET-02）：`drive_review_session_pass_enters_author_confirm` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

#[tokio::test]
async fn drive_review_session_strong_revise_returns_to_author_confirm() {
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

    // spec-design-dialog-revision T5：Story 强 revise 完成不再停在 ReviewDecision，统一回 AuthorConfirm。
    assert_eq!(engine.session().stage, WorkspaceStage::AuthorConfirm);
    match engine.build_session_state() {
        WsOutMessage::SessionState {
            timeline_nodes,
            active_node_id,
            ..
        } => {
            let active = timeline_nodes
                .iter()
                .find(|node| Some(&node.node_id) == active_node_id.as_ref())
                .expect("active author confirm node");
            assert_eq!(active.node_type, TimelineNodeType::AuthorConfirm);
            assert_eq!(active.status, TimelineNodeStatus::Active);
        }
        _ => panic!("expected SessionState"),
    }
}

use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone)]
struct QueuedReviewProvider {
    outputs: Arc<Mutex<VecDeque<String>>>,
    prompts: Arc<Mutex<Vec<String>>>,
    resume_provider_session_ids: Arc<Mutex<Vec<Option<String>>>>,
    starts: Arc<AtomicUsize>,
}

impl QueuedReviewProvider {
    fn new(outputs: Vec<String>) -> Self {
        Self {
            outputs: Arc::new(Mutex::new(outputs.into())),
            prompts: Arc::new(Mutex::new(Vec::new())),
            resume_provider_session_ids: Arc::new(Mutex::new(Vec::new())),
            starts: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for QueuedReviewProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let start = self.starts.fetch_add(1, Ordering::SeqCst) + 1;
        self.prompts.lock().unwrap().push(input.prompt.clone());
        self.resume_provider_session_ids
            .lock()
            .unwrap()
            .push(input.resume_provider_session_id.clone());
        let template = self
            .outputs
            .lock()
            .unwrap()
            .pop_front()
            .expect("queued review output");
        let output = input
            .structured_output_contract
            .as_ref()
            .map(|contract| template.replace("__NONCE__", &contract.nonce))
            .unwrap_or(template);
        let completion = ProviderCompletion::from_output(
            output,
            input.structured_output_contract.as_ref(),
            Some(format!("review-session-{start}")),
        );
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(4);
        event_tx
            .send(ProviderEvent::Completed(completion))
            .await
            .unwrap();
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

async fn queued_review_engine(
    session_id: &str,
) -> (
    TempDir,
    WorkspaceEngine,
    mpsc::Receiver<EngineEvent>,
    String,
) {
    queued_review_engine_for(
        session_id,
        WorkspaceType::Story,
        artifact_payload("# Story Spec\n\n需要审核的候选版本"),
    )
    .await
}

async fn queued_review_engine_for(
    session_id: &str,
    workspace_type: WorkspaceType,
    artifact: ArtifactPayload,
) -> (
    TempDir,
    WorkspaceEngine,
    mpsc::Receiver<EngineEvent>,
    String,
) {
    let (tmp, store) = setup();
    let (tx, rx) = mpsc::channel(64);
    let mut session = make_session(session_id);
    session.entity_id = match workspace_type {
        WorkspaceType::Story => "story_spec_0001",
        WorkspaceType::Design => "design_spec_0001",
        WorkspaceType::WorkItem => "work_item_0001",
        WorkspaceType::WorkItemPlan => "work_item_plan_0001",
    }
    .to_string();
    session.workspace_type = workspace_type;
    session.artifact = Some(artifact);
    let mut engine = WorkspaceEngine::new(store, tx, session);
    engine.start_review().await;
    let review_node_id = engine
        .active_node_id
        .clone()
        .expect("active review node id");
    (tmp, engine, rx, review_node_id)
}

fn repair_event_statuses(
    rx: &mut mpsc::Receiver<EngineEvent>,
) -> Vec<(ProviderExecutionEventStatus, Option<String>)> {
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        if let EngineEvent::ExecutionEvent { event, node_id, .. } = event
            && event.event_id == "structured_output_repair"
        {
            events.push((event.status, node_id));
        }
    }
    events
}

fn missing_json_nonce_output(json: &str) -> String {
    format!(
        "审核发现需要返修。\n<ARIA_STRUCTURED_OUTPUT nonce=\"__NONCE__\">{json}</ARIA_STRUCTURED_OUTPUT>"
    )
}

fn valid_structured_output(json: &str) -> String {
    let json = json.replacen('{', r#"{"nonce":"__NONCE__","#, 1);
    format!(
        "格式修复完成。\n<ARIA_STRUCTURED_OUTPUT nonce=\"__NONCE__\">{json}</ARIA_STRUCTURED_OUTPUT>"
    )
}

#[tokio::test]
async fn kimi_review_repairs_missing_json_nonce_once_for_all_workspace_types() {
    enum KimiReviewRepairCase {
        General {
            name: &'static str,
            workspace_type: WorkspaceType,
            artifact: ArtifactPayload,
            review_json: &'static str,
        },
        WorkItemPlan,
    }

    let general_review_json = r#"{"verdict":"revise","summary":"补充失败路径","findings":[{"severity":"must_fix","message":"缺少失败路径","evidence":"当前产物遗漏","required_action":"补充失败路径"}]}"#;
    let cases = [
        KimiReviewRepairCase::General {
            name: "story",
            workspace_type: WorkspaceType::Story,
            artifact: artifact_payload(&complete_story_artifact(
                "补充格式修复",
                "修复后可继续审核",
            )),
            review_json: general_review_json,
        },
        KimiReviewRepairCase::General {
            name: "design",
            workspace_type: WorkspaceType::Design,
            artifact: artifact_payload(&complete_design_artifact(
                "保持业务 payload",
                "复用 reviewer session",
            )),
            review_json: general_review_json,
        },
        KimiReviewRepairCase::General {
            name: "work_item",
            workspace_type: WorkspaceType::WorkItem,
            artifact: artifact_payload(&complete_work_item_artifact("修复 reviewer 结构化输出")),
            review_json: general_review_json,
        },
        KimiReviewRepairCase::WorkItemPlan,
    ];

    for case in cases {
        let (_tmp, case_name, review_json, mut engine, mut rx, review_node_id) = match case {
            KimiReviewRepairCase::General {
                name,
                workspace_type,
                artifact,
                review_json,
            } => {
                let (_tmp, engine, rx, review_node_id) = queued_review_engine_for(
                    &format!("sess_kimi_review_repair_{name}"),
                    workspace_type,
                    artifact,
                )
                .await;
                (_tmp, name, review_json, engine, rx, review_node_id)
            }
            KimiReviewRepairCase::WorkItemPlan => {
                let (_tmp, engine, rx, review_node_id) = queued_work_item_plan_outline_review_engine(
                    "sess_kimi_review_repair_work_item_plan",
                )
                .await;
                (
                    _tmp,
                    "work_item_plan",
                    work_item_plan_outline_revise_json(),
                    engine,
                    rx,
                    review_node_id,
                )
            }
        };
        let provider = QueuedReviewProvider::new(vec![
            missing_json_nonce_output(review_json),
            valid_structured_output(review_json),
        ]);
        engine.session.reviewer_provider = Some(ProviderName::KimiCode);

        engine
            .drive_review_session(Arc::new(provider.clone()), empty_provider_commands())
            .await;

        assert_eq!(provider.starts.load(Ordering::SeqCst), 2, "{case_name}");
        assert_eq!(
            provider.resume_provider_session_ids.lock().unwrap()[1],
            Some("review-session-1".to_string()),
            "{case_name}"
        );
        let diagnostic = engine
            .latest_review_verdict
            .as_ref()
            .and_then(|verdict| verdict.structured_output_diagnostic.as_ref())
            .expect("repair diagnostic");
        assert!(diagnostic.repair_attempted, "{case_name}");
        assert!(diagnostic.repair_succeeded, "{case_name}");
        assert_eq!(
            repair_event_statuses(&mut rx),
            vec![
                (
                    ProviderExecutionEventStatus::Started,
                    Some(review_node_id.clone())
                ),
                (ProviderExecutionEventStatus::Completed, Some(review_node_id)),
            ],
            "{case_name}"
        );
    }
}

#[tokio::test]
async fn review_structured_output_repair_failure_persists_diagnostic() {
    let review_json = r#"{"verdict":"revise","summary":"补充失败路径","findings":[{"severity":"must_fix","message":"缺少失败路径","evidence":"当前产物遗漏","required_action":"补充失败路径"}]}"#;
    let provider = QueuedReviewProvider::new(vec![
        missing_json_nonce_output(review_json),
        missing_json_nonce_output(review_json),
    ]);
    let (_tmp, mut engine, mut rx, review_node_id) =
        queued_review_engine("sess_review_repair_failure").await;

    engine
        .drive_review_session(Arc::new(provider.clone()), empty_provider_commands())
        .await;

    assert_eq!(provider.starts.load(Ordering::SeqCst), 2);
    let verdict = engine.latest_review_verdict.as_ref().expect("fallback verdict");
    assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
    let diagnostic = verdict
        .structured_output_diagnostic
        .as_ref()
        .expect("repair failure diagnostic");
    assert_eq!(diagnostic.code, "missing_json_nonce");
    assert!(diagnostic.repair_attempted);
    assert!(!diagnostic.repair_succeeded);
    assert!(
        diagnostic
            .raw_output_preview
            .as_ref()
            .expect("raw output preview")
            .chars()
            .count()
            <= 2_048
    );
    let repair_events = repair_event_statuses(&mut rx);
    assert_eq!(
        repair_events.last(),
        Some(&(
            ProviderExecutionEventStatus::Failed,
            Some(review_node_id)
        ))
    );
}

#[tokio::test]
async fn review_structured_output_repair_rejects_payload_change() {
    let first_json = r#"{"verdict":"revise","summary":"必须修复","findings":[{"severity":"must_fix","message":"缺少失败路径","evidence":"当前产物遗漏","required_action":"补充失败路径"}]}"#;
    let changed_json = r#"{"verdict":"pass","summary":"可以确认","findings":[]}"#;
    let provider = QueuedReviewProvider::new(vec![
        missing_json_nonce_output(first_json),
        valid_structured_output(changed_json),
    ]);
    let (_tmp, mut engine, mut rx, review_node_id) =
        queued_review_engine("sess_review_repair_payload_changed").await;
    engine.session.reviewer_provider = Some(ProviderName::KimiCode);

    engine
        .drive_review_session(Arc::new(provider.clone()), empty_provider_commands())
        .await;

    assert_eq!(provider.starts.load(Ordering::SeqCst), 2);
    let verdict = engine.latest_review_verdict.as_ref().expect("fallback verdict");
    assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
    let diagnostic = verdict
        .structured_output_diagnostic
        .as_ref()
        .expect("payload change diagnostic");
    assert_eq!(diagnostic.code, "repair_payload_changed");
    assert!(diagnostic.repair_attempted);
    assert!(!diagnostic.repair_succeeded);
    let repair_events = repair_event_statuses(&mut rx);
    assert_eq!(
        repair_events.last(),
        Some(&(
            ProviderExecutionEventStatus::Failed,
            Some(review_node_id)
        ))
    );
}

// C2 改写登记（REQ-HGC-03 场景 2）：invalid_json 现在获得同 invocation 恰一次
// 静默重试（同一 input 重发，非 repair prompt）；重试仍失败保原始 diagnostic
// 进人工。「不触发不可验证 repair」的原语义保留——重试不是 payload-equality
// repair，成功消费的前提是重试输出自身完整可解析。
#[tokio::test]
async fn invalid_review_json_retries_same_input_without_verifiable_payload_repair() {
    let provider = QueuedReviewProvider::new(vec![
        valid_structured_output(r#"{"verdict":"pass","summary":}"#),
        valid_structured_output(r#"{"verdict":"pass","summary":}"#),
    ]);
    let (_tmp, mut engine, mut rx, _review_node_id) =
        queued_review_engine("sess_review_invalid_json_no_repair").await;

    engine
        .drive_review_session(Arc::new(provider.clone()), empty_provider_commands())
        .await;

    assert_eq!(provider.starts.load(Ordering::SeqCst), 2);
    let prompts = provider.prompts.lock().unwrap().clone();
    assert_eq!(prompts[0], prompts[1], "retry resends the original input");
    let verdict = engine.latest_review_verdict.as_ref().expect("fallback verdict");
    assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
    let diagnostic = verdict
        .structured_output_diagnostic
        .as_ref()
        .expect("invalid json diagnostic");
    assert_eq!(diagnostic.code, "invalid_json");
    assert!(diagnostic.repair_attempted);
    assert!(!diagnostic.repair_succeeded);
    assert!(repair_event_statuses(&mut rx).is_empty());
}

// —— C2 human-gate-convergence（REQ-HGC-03 场景 2/F-52 R6）——
//
// 现场（issue_0002/workspace_session_0009 R6）：reviewer raw output 实为
// pass+3 advisory，仅结构化 JSON 非法（invalid_json）——整轮被丢弃降级
// needs_human，白耗一轮 + 3h 门等待。新契约：解析失败（invalid_json 等
// Syntax 族）在同 invocation 内恰一次静默重试（同一 input、不发事件、
// 不记账）；重试成功正常消费；仍失败保原始 diagnostic 进人工，且该轮
// 不计为一次完整 review（review/cycle/返修计数零增量）。
#[tokio::test]
async fn invalid_json_retries_once_within_invocation_and_consumes_recovery() {
    let provider = QueuedReviewProvider::new(vec![
        valid_structured_output(r#"{"verdict":"pass","summary":}"#),
        valid_structured_output(r#"{"verdict":"pass","summary":"格式恢复","findings":[]}"#),
    ]);
    let (_tmp, mut engine, mut rx, _review_node_id) =
        queued_review_engine("sess_review_invalid_json_retry_success").await;

    engine
        .drive_review_session(Arc::new(provider.clone()), empty_provider_commands())
        .await;

    assert_eq!(provider.starts.load(Ordering::SeqCst), 2);
    // 同 invocation 静默重试：同一份 input 重新拉起（非 repair prompt），
    // 续用同一 provider 会话。
    let prompts = provider.prompts.lock().unwrap().clone();
    assert_eq!(prompts[0], prompts[1], "retry must resend the original input");
    assert!(
        !prompts[1].contains("结构化输出格式无效"),
        "retry must not build a repair prompt"
    );
    assert_eq!(
        provider.resume_provider_session_ids.lock().unwrap()[1],
        Some("review-session-1".to_string())
    );
    let verdict = engine
        .latest_review_verdict
        .as_ref()
        .expect("recovered verdict");
    assert_eq!(verdict.verdict, ReviewVerdictType::Pass);
    let diagnostic = verdict
        .structured_output_diagnostic
        .as_ref()
        .expect("retry diagnostic");
    assert_eq!(diagnostic.code, "invalid_json");
    assert!(diagnostic.repair_attempted);
    assert!(diagnostic.repair_succeeded);
    // 静默纪律：不发 structured_output_repair 执行事件。
    assert!(repair_event_statuses(&mut rx).is_empty());
}

#[tokio::test]
async fn invalid_json_retry_failure_preserves_diagnostic_and_counts_nothing() {
    let provider = QueuedReviewProvider::new(vec![
        valid_structured_output(r#"{"verdict":"pass","summary":}"#),
        valid_structured_output(r#"{"verdict":"pass","summary":}"#),
    ]);
    let (_tmp, mut engine, _rx, _review_node_id) =
        queued_work_item_plan_outline_review_engine("sess_review_invalid_json_retry_failed")
            .await;

    engine
        .drive_review_session(Arc::new(provider.clone()), empty_provider_commands())
        .await;

    // 恰一次重试：两次 provider 启动后不再追加第三次。
    assert_eq!(provider.starts.load(Ordering::SeqCst), 2);
    let verdict = engine
        .latest_review_verdict
        .as_ref()
        .expect("fallback verdict");
    assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
    let diagnostic = verdict
        .structured_output_diagnostic
        .as_ref()
        .expect("degraded diagnostic");
    assert_eq!(diagnostic.code, "invalid_json");
    assert!(diagnostic.repair_attempted);
    assert!(!diagnostic.repair_succeeded);
    assert!(diagnostic.raw_output_preview.is_some());
    // 零计数增量×3（REQ-HGC-03）：review 计数（含 cycle）/自动返修/人工预算
    // ——降级轮不计为一次完整 review，人工门以默认预算开门。
    assert_eq!(engine.session.run_history.initial_review_count, 0);
    assert!(engine.session.run_history.review_cycles.is_empty());
    assert_eq!(engine.session.run_history.repairs_used, 0);
    let gate = engine
        .session
        .human_gate_snapshot
        .as_ref()
        .expect("degraded round opens the human gate");
    assert_eq!(gate.manual_repairs_remaining, 3);
    assert_eq!(gate.accepted_feedback_turns, Some(0));
}

#[tokio::test]
async fn malformed_review_findings_do_not_trigger_business_rewrite() {
    let provider = QueuedReviewProvider::new(vec![valid_structured_output(
        r#"{"verdict":"revise","summary":"findings 非法","findings":[{"severity":"must_fix","message":42}]}"#,
    )]);
    let (_tmp, mut engine, _rx, _review_node_id) =
        queued_review_engine("sess_review_malformed_findings_no_repair").await;

    engine
        .drive_review_session(Arc::new(provider.clone()), empty_provider_commands())
        .await;

    assert_eq!(provider.starts.load(Ordering::SeqCst), 1);
    let verdict = engine.latest_review_verdict.as_ref().expect("fallback verdict");
    assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
    assert_eq!(
        verdict
            .structured_output_diagnostic
            .as_ref()
            .expect("malformed findings diagnostic")
            .code,
        "malformed_findings"
    );
}
