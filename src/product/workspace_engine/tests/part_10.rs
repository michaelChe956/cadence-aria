#[test]
fn artifact_constraints_require_visible_source_id_traceability() {
    let story = validate_workspace_artifact_constraints(
        "# Story Spec\n\n\
         ## 范围\n覆盖基础流程。\n\n\
         ## 用户故事\n作为用户，我要完成操作。\n\n\
         ## 功能需求\n- [REQ-001] 系统支持操作。\n\n\
         ## 成功标准\n- [AC-001] 操作成功。\n\n\
         ## 待确认项\n无。\n\n\
         ## 非功能需求\n无。\n",
        &WorkspaceType::Story,
    );
    assert!(!story.passed);
    assert!(
        story
            .missing_required_ids
            .iter()
            .any(|missing| missing == "source id")
    );

    let design = validate_workspace_artifact_constraints(
        "# Design Spec\n\n\
         ## 设计范围\n覆盖设计。\n\n\
         ## 设计决策\n- [DEC-001] 采用现有架构。\n\n\
         ## 公共组件\n- [CMP-001] WorkspaceContext。\n\n\
         ## API 契约\n- [API-001] 保持现有接口。\n\n\
         ## 数据模型\n无新增。\n\n\
         ## 风险\n无。\n\n\
         ## 追踪关系\n- [DEC-001] -> [REQ-001]\n",
        &WorkspaceType::Design,
    );
    assert!(!design.passed);
    assert!(
        design
            .missing_required_ids
            .iter()
            .any(|missing| missing == "source id")
    );

    let work_item = validate_workspace_artifact_constraints(
        "# Work Item\n\n\
         ## 目标\n实现当前任务。\n\n\
         ## 范围\n仅当前任务。\n\n\
         ## 实现步骤\n- 接入接口。\n\n\
         ## 依赖\n无。\n\n\
         ## 验证命令\ncargo test --locked --lib current_task。\n\n\
         ## 风险\n无。\n\n\
         ## 追踪关系\n- [REQ-001]\n",
        &WorkspaceType::WorkItem,
    );
    assert!(!work_item.passed);
    assert!(
        work_item
            .missing_required_ids
            .iter()
            .any(|missing| missing == "source id")
    );

    let work_item_plan = validate_workspace_artifact_constraints(
        "# Work Item Plan\n\n\
         ## 计划范围\n覆盖 Issue。\n\n\
         ## 任务拆分\n- [TASK-001] 后端。\n\n\
         ## 依赖图\n无。\n\n\
         ## 验证计划\ncargo test --locked。\n\n\
         ## 执行顺序\n先后端。\n\n\
         ## 风险\n无。\n\n\
         ## 追踪关系\n[TASK-001] -> [REQ-001]\n",
        &WorkspaceType::WorkItemPlan,
    );
    assert!(!work_item_plan.passed);
    assert!(
        work_item_plan
            .missing_required_ids
            .iter()
            .any(|missing| missing == "source id")
    );
}

#[test]
fn story_artifact_accepts_explicit_issue_id_traceability_without_literal_source_id() {
    let report = validate_workspace_artifact_constraints(
        "# Aria Provider Setup Story Spec\n\n\
         ## 范围\n**来源**：Issue `issue_0001` — provider 安装引导。\n\n\
         ## 用户故事\n作为用户，我要完成 provider 安装。\n\n\
         ## 功能需求\n- [REQ-001] 系统支持 provider 检查。\n\n\
         ## 成功标准\n- [AC-001] 用户能看到 provider 状态。\n\n\
         ## 待确认项\n无。\n\n\
         ## 非功能需求\n无。\n",
        &WorkspaceType::Story,
    );

    assert!(report.passed, "{report:?}");
}

#[test]
fn story_artifact_rejects_nested_artifact_fence_and_thinking_pollution() {
    let report = validate_workspace_artifact_constraints(
        "# Aria Provider Setup Story Spec\n\n\
         ## 范围\n**来源**：Issue `issue_0001` — provider 安装引导。\n\
         ```\n\n\
         <thinking>\nNow I will continue.\n</thinking>\n\n\
         ```artifact\n\
         ## 用户故事\n作为用户，我要完成 provider 安装。\n\n\
         ## 功能需求\n- [REQ-001] 系统支持 provider 检查。\n\n\
         ## 成功标准\n- [AC-001] 用户能看到 provider 状态。\n\n\
         ## 待确认项\n无。\n\n\
         ## 非功能需求\n无。\n",
        &WorkspaceType::Story,
    );

    assert!(!report.passed);
    let reasons = report.blocking_reasons();
    assert!(
        reasons.iter().any(|reason| reason.contains("artifact fence")),
        "{reasons:?}"
    );
    assert!(
        reasons.iter().any(|reason| reason.contains("<thinking>")),
        "{reasons:?}"
    );
}

#[test]
fn story_artifact_rejects_unresolved_open_items_without_interaction() {
    let report = validate_workspace_artifact_constraints(
        "# Aria Provider Setup Story Spec\n\n\
         ## 范围\n**来源**：Issue `issue_0001` — provider 安装引导。\n\n\
         ## 用户故事\n作为用户，我要完成 provider 安装。\n\n\
         ## 功能需求\n- [REQ-001] 系统支持 provider 检查。\n\n\
         ## 成功标准\n- [AC-001] 用户能看到 provider 状态。\n\n\
         ## 待确认项\n**[OPEN-001]** Codex 的 npm 包名需要确认。\n\n\
         ## 非功能需求\n无。\n",
        &WorkspaceType::Story,
    );

    assert!(!report.passed);
    let reasons = report.blocking_reasons();
    assert!(
        reasons
            .iter()
            .any(|reason| reason.contains("待确认项") && reason.contains("AskUserQuestion")),
        "{reasons:?}"
    );
}

#[test]
fn story_artifact_accepts_resolved_open_items_with_confirmation_note() {
    let report = validate_workspace_artifact_constraints(
        "# Aria Provider Setup Story Spec\n\n\
         ## 范围\n**来源**：Issue `issue_0001` — provider 安装引导。\n\n\
         ## 用户故事\n作为用户，我要完成 provider 安装。\n\n\
         ## 功能需求\n- [REQ-001] 系统支持 provider 检查。\n\n\
         ## 成功标准\n- [AC-001] 用户能看到 provider 状态。\n\n\
         ## 待确认项\n无。所有影响范围与验收标准的未决点已通过结构化交互确认（claude code 必装且阻断、记录存全局用户目录、首次强制+后续静默复核不弹窗、首版仅两个 provider）。实现细节（如 npm 具体包名、全局配置目录确切路径、安装命令执行方式）留给 Design 阶段决定，不属于本 Story Spec 的未决项。\n\n\
         ## 非功能需求\n无。\n",
        &WorkspaceType::Story,
    );

    assert!(
        report.passed,
        "resolved confirmation note should not be treated as an open item: {:?}",
        report.blocking_reasons()
    );
}

#[test]
fn story_artifact_still_rejects_no_prefix_followed_by_real_open_item() {
    let report = validate_workspace_artifact_constraints(
        "# Aria Provider Setup Story Spec\n\n\
         ## 范围\n**来源**：Issue `issue_0001` — provider 安装引导。\n\n\
         ## 用户故事\n作为用户，我要完成 provider 安装。\n\n\
         ## 功能需求\n- [REQ-001] 系统支持 provider 检查。\n\n\
         ## 成功标准\n- [AC-001] 用户能看到 provider 状态。\n\n\
         ## 待确认项\n无。Codex 的 npm 包名仍待确认。\n\n\
         ## 非功能需求\n无。\n",
        &WorkspaceType::Story,
    );

    assert!(!report.passed);
    let reasons = report.blocking_reasons();
    assert!(
        reasons
            .iter()
            .any(|reason| reason.contains("待确认项") && reason.contains("AskUserQuestion")),
        "{reasons:?}"
    );
}

#[test]
fn artifact_retry_prompt_includes_validation_reasons() {
    let previous_output = "# Story Spec\n\n## 范围\n缺少其余章节。";
    let reasons = vec![
        "缺少 heading: 用户故事".to_string(),
        "缺少 source id".to_string(),
    ];

    let prompt = build_artifact_retry_prompt(&WorkspaceType::Story, previous_output, &reasons);

    assert!(prompt.contains("具体失败原因"));
    assert!(prompt.contains("缺少 heading: 用户故事"));
    assert!(prompt.contains("缺少 source id"));
    assert!(prompt.contains("只能输出一个完整 artifact fenced block"));
    assert!(prompt.contains("AskUserQuestion"));
    assert!(prompt.contains("待确认项"));
    assert!(prompt.contains("结构化交互"));
    assert!(prompt.contains("用户确认决策"));
    assert!(prompt.contains("author-decision"));
    assert!(prompt.contains("[REQ-"));
    assert!(prompt.contains("[AC-"));
}

#[tokio::test]
async fn automatic_artifact_retry_uses_separate_timeline_node_for_retry_stream() {
    let (_tmp, lifecycle_store, mut engine) = persistent_test_engine();
    engine.session.reviewer_provider = None;
    let provider = Arc::new(StoryOpenItemRetryProvider::default());

    engine
        .handle_user_message(
            "开始生成 Story Spec".to_string(),
            provider.clone(),
            empty_provider_commands(),
        )
        .await;

    assert_eq!(*provider.calls.lock().unwrap(), 2);
    let author_nodes = engine
        .timeline_nodes
        .iter()
        .filter(|node| node.node_type == TimelineNodeType::AuthorRun)
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(author_nodes.len(), 2, "{author_nodes:?}");
    assert_eq!(author_nodes[0].status, TimelineNodeStatus::Failed);
    assert!(
        author_nodes[0]
            .summary
            .as_deref()
            .is_some_and(|summary| summary.contains("待确认项")),
        "{author_nodes:?}"
    );
    assert_eq!(author_nodes[1].status, TimelineNodeStatus::Completed);
    assert_eq!(
        author_nodes[1]
            .retry
            .as_ref()
            .map(|retry| retry.retry_of_node_id.as_str()),
        Some(author_nodes[0].node_id.as_str())
    );

    let original_detail = lifecycle_store
        .load_node_detail(&engine.session().session_id, &author_nodes[0].node_id)
        .expect("original node detail");
    let retry_detail = lifecycle_store
        .load_node_detail(&engine.session().session_id, &author_nodes[1].node_id)
        .expect("retry node detail");
    assert!(original_detail.streaming_content.contains("[OPEN-001]"));
    assert!(!original_detail.streaming_content.contains("# Retried Story Spec"));
    assert!(retry_detail.streaming_content.contains("# Retried Story Spec"));
    assert!(!retry_detail.streaming_content.contains("[OPEN-001]"));

    let inputs = provider.inputs.lock().unwrap();
    assert_eq!(inputs.len(), 2);
    assert!(
        inputs[1].prompt.contains("具体失败原因")
            && inputs[1].prompt.contains("待确认项")
            && inputs[1].prompt.contains("AskUserQuestion"),
        "{}",
        inputs[1].prompt
    );
}

#[derive(Default)]
struct StoryOpenItemRetryProvider {
    inputs: Arc<Mutex<Vec<StreamingProviderInput>>>,
    calls: Arc<Mutex<u32>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for StoryOpenItemRetryProvider {
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
        tokio::spawn(async move {
            let output = if call_no == 1 {
                "```artifact\n# Story Spec\n\n\
                 ## 范围\n来源 source id: Issue issue_0001。\n\n\
                 ## 用户故事\n作为用户，我希望完成 provider 安装。\n\n\
                 ## 功能需求\n- [REQ-001] 系统支持 provider 检查。\n\n\
                 ## 成功标准\n- [AC-001] 用户能看到 provider 状态。\n\n\
                 ## 待确认项\n**[OPEN-001]** Codex 的 npm 包名需要确认。\n\n\
                 ## 非功能需求\n无。\n```"
                    .to_string()
            } else {
                format!(
                    "```artifact\n{}```",
                    complete_story_artifact("已通过交互确认 provider 包名。", "不再保留待确认项。")
                        .replacen("# Story Spec", "# Retried Story Spec", 1)
                )
            };
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: output.clone(),
                })
                .await;
            let _ = event_tx
                .send(ProviderEvent::Completed {
                    full_output: output,
                    provider_session_id: Some(format!("story-open-item-retry-{call_no}")),
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
