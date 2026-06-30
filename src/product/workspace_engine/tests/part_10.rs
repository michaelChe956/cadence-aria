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
