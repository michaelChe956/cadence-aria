// part_02 拆分（large_file_guard 1200 行上限）：artifact 约束与 review input 用例族经 include! 挂载，模块作用域与用例名不变，行为零变化。
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
fn story_artifact_with_inline_code_ids_passes_validation() {
    let report = validate_workspace_artifact_constraints(
        "# Story Spec\n\n\
         ## 范围\n来源 source id: Issue issue_0001；覆盖基础流程。\n\n\
         ## 用户故事\n作为用户，我要完成操作。\n\n\
         ## 功能需求\n- `[REQ-001]` 系统支持操作。\n\n\
         ## 成功标准\n- `[AC-001]` 操作成功。\n\n\
         ## 待确认项\n无。\n\n\
         ## 非功能需求\n无。\n",
        &WorkspaceType::Story,
    );
    assert!(
        report.passed,
        "inline code 包裹的 [REQ-*]/[AC-*] 应通过校验: {:?}",
        report.blocking_reasons()
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
        structured_output_diagnostic: None,
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
            .contains("\"verdict\":\"pass|revise|needs_human\"")
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
            "Design artifact: Work Item Plan、开发任务列表、任务拆分、测试计划、测试范围或场景、测试文件或模块、测试框架或夹具、测试命令、构建命令、执行 checklist 或将测试或验证职责分配给组件或文件必须报告为 must_fix；仅把 [DEC-*] 关联到 [REQ-*]/[AC-*] 且不描述如何测试的抽象验收可追踪性不得报告为 must_fix。",
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
