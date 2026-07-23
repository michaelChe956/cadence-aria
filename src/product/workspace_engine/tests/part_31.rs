#[test]
fn initial_author_inputs_directly_route_every_workspace_artifact_type() {
    for (workspace_type, required_skill) in [
        (WorkspaceType::Story, "using-superpowers → brainstorming"),
        (WorkspaceType::Design, "using-superpowers → brainstorming"),
        (WorkspaceType::WorkItemPlan, "using-superpowers → writing-plans"),
        (WorkspaceType::WorkItem, "using-superpowers → writing-plans"),
    ] {
        let (event_tx, _event_rx) = mpsc::channel(8);
        let mut session = make_session(&format!("sess_initial_author_route_{workspace_type:?}"));
        session.workspace_type = workspace_type.clone();
        let checkpoint_tmp = TempDir::new().unwrap();
        let engine = WorkspaceEngine::new(
            Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
            event_tx,
            session,
        );

        let prompt = engine
            .build_streaming_input("开始生成", AuthorPromptMode::FullConversation)
            .expect("author input")
            .prompt;

        assert!(
            prompt.contains("[cadence_original_routing_rules]"),
            "initial {workspace_type:?} author prompt must include the direct routing reference: {prompt}"
        );
        assert_eq!(
            prompt.matches("[cadence_original_routing_rules]").count(),
            1,
            "initial {workspace_type:?} author fallback must include the direct routing reference exactly once: {prompt}"
        );
        assert!(
            prompt.contains("agent-routing-kernel.md")
                && prompt.contains("openspec-superpowers-workflow.md"),
            "initial {workspace_type:?} author prompt must name both authoritative rule files: {prompt}"
        );
        assert!(
            prompt.contains(required_skill),
            "initial {workspace_type:?} author prompt must select its phase skill: {prompt}"
        );
    }
}

#[test]
fn initial_author_reuses_system_routing_reference_for_every_workspace_type() {
    for workspace_type in [
        WorkspaceType::Story,
        WorkspaceType::Design,
        WorkspaceType::WorkItem,
        WorkspaceType::WorkItemPlan,
    ] {
        let (event_tx, _event_rx) = mpsc::channel(8);
        let mut session = make_session(&format!("sess_initial_author_reuse_{workspace_type:?}"));
        session.workspace_type = workspace_type.clone();
        session.messages.push(SessionMessage {
            id: "msg_generation_context".to_string(),
            role: "system".to_string(),
            content: format!(
                "[workflow_discipline]\n{}\n[output_schema]\n来自正常 generation brief 的合同。",
                crate::product::cadence_skills::routing_reference::direct_cadence_routing_rules_reference()
            ),
            checkpoint_id: None,
            created_at: "2026-07-23T00:00:00Z".to_string(),
        });
        let checkpoint_tmp = TempDir::new().unwrap();
        let engine = WorkspaceEngine::new(
            Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
            event_tx,
            session,
        );

        let prompt = engine
            .build_streaming_input("开始生成", AuthorPromptMode::FullConversation)
            .expect("author input")
            .prompt;

        assert_eq!(
            prompt.matches("[cadence_original_routing_rules]").count(),
            1,
            "initial {workspace_type:?} author prompt must reuse, not repeat, the system routing reference: {prompt}"
        );
    }
}

#[test]
fn review_input_keeps_generation_context_without_author_workflow_directives() {
    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut session = make_session("sess_review_uses_canonical_generation_context");
    session.messages = vec![
        SessionMessage {
            id: "msg_001".to_string(),
            role: "system".to_string(),
            content: concat!(
                "Workspace 生成任务已准备\n\n",
                "[system]\n你是 Aria 的候选 Story Spec 生成器。\n\n",
                "[node_contract]\nadapter_role=orchestrator\n\n",
                "[canonical_inputs]\n",
                "Issue 描述: reviewer 必须保留这一需求上下文。\n\n",
                "[constraint_summary]\n- 必须保留现有兼容性约束。\n\n",
                "[runtime_contract]\n作者运行时契约。\n\n",
                "[workflow_discipline]\n",
                "当前阶段：候选产物 author。\n",
                "必调 Skill：using-superpowers → brainstorming。\n\n",
                "[output_schema]\n作者产物结构。\n\n",
                "[completion_or_failure]\n作者完成条件。"
            )
            .to_string(),
            checkpoint_id: None,
            created_at: "2026-06-01T00:00:00Z".to_string(),
        },
        SessionMessage {
            id: "msg_002".to_string(),
            role: "user".to_string(),
            content: "用户补充：审核时必须保留边界。".to_string(),
            checkpoint_id: None,
            created_at: "2026-06-01T00:00:01Z".to_string(),
        },
    ];
    session.artifact = Some(artifact_payload(
        "# Current Story Spec\n\n## 功能需求\n- [REQ-001] 当前稿。\n\n## 成功标准\n- [AC-001] 当前稿可审核。\n",
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
            .contains("Issue 描述: reviewer 必须保留这一需求上下文。"),
        "reviewer must retain canonical generation context: {}",
        input.prompt
    );
    assert!(input.prompt.contains("必须保留现有兼容性约束。"));
    assert!(input.prompt.contains("当前阶段：候选产物审查。"));
    assert!(
        !input
            .prompt
            .contains("必调 Skill：using-superpowers → brainstorming。"),
        "reviewer must not reactivate the historical author workflow: {}",
        input.prompt
    );
    assert!(!input.prompt.contains("候选 Story Spec 生成器。"));
    assert!(!input.prompt.contains("adapter_role=orchestrator"));
    assert!(!input.prompt.contains("作者运行时契约。"));
    assert!(!input.prompt.contains("作者产物结构。"));
    assert!(!input.prompt.contains("作者完成条件。"));
}

#[test]
fn design_reviewer_boundary_distinguishes_traceability_from_executable_testing() {
    let rules = reviewer_boundary_rules_for(&WorkspaceType::Design);

    assert!(rules.contains("抽象验收可追踪性"), "{rules}");
    assert!(rules.contains("不得报告为 must_fix"), "{rules}");
    for forbidden in [
        "测试计划",
        "测试范围或场景",
        "测试文件或模块",
        "测试框架或夹具",
        "测试命令",
        "构建命令",
        "执行 checklist",
        "将测试或验证职责分配给组件或文件",
    ] {
        assert!(rules.contains(forbidden), "missing `{forbidden}`: {rules}");
    }
}

#[test]
fn initial_author_prompts_render_parser_derived_schema() {
    for workspace_type in [
        WorkspaceType::Story,
        WorkspaceType::Design,
        WorkspaceType::WorkItem,
    ] {
        let (_tmp, store) = setup();
        let (event_tx, _event_rx) = mpsc::channel(8);
        let mut session = make_session(&format!("sess_schema_author_{workspace_type:?}"));
        session.workspace_type = workspace_type.clone();
        let engine = WorkspaceEngine::new(store, event_tx, session);

        let prompt = engine
            .build_streaming_input("开始生成", AuthorPromptMode::FullConversation)
            .expect("author input")
            .prompt;
        let spec = artifact_constraint_spec_for(&workspace_type);

        assert!(
            prompt.contains("[artifact_schema_contract]"),
            "{workspace_type:?} author prompt must expose the parser-derived schema: {prompt}"
        );
        assert!(
            prompt.contains("缺一不可"),
            "{workspace_type:?} author prompt must make schema completeness explicit: {prompt}"
        );
        for label in spec
            .required_headings
            .iter()
            .map(|rule| rule.label)
            .chain(spec.required_id_patterns.iter().map(|rule| rule.label))
            .chain(spec.required_tokens.iter().map(|rule| rule.label))
        {
            assert!(
                prompt.contains(label),
                "{workspace_type:?} author prompt must include required parser label `{label}`: {prompt}"
            );
        }
    }
}

#[test]
fn parser_derived_schema_contract_keeps_concrete_heading_and_id_examples() {
    for workspace_type in [
        WorkspaceType::Story,
        WorkspaceType::Design,
        WorkspaceType::WorkItem,
    ] {
        let schema = author_artifact_schema_contract_for(&workspace_type)
            .expect("Markdown workspace must have a schema contract");
        let spec = artifact_constraint_spec_for(&workspace_type);

        for heading in &spec.required_headings {
            assert!(
                schema.contains(&format!("## {}", heading.label)),
                "{workspace_type:?} schema must render parser heading `{}` as a concrete Markdown heading: {schema}",
                heading.label
            );
        }
        for id_rule in &spec.required_id_patterns {
            let example = match id_rule.pattern {
                ArtifactTokenPattern::BracketPrefix(prefix) => format!("[{prefix}001]"),
                ArtifactTokenPattern::WordPrefix(prefix) => format!("{prefix}001"),
                ArtifactTokenPattern::Literal(literal) => literal.to_string(),
            };
            assert!(
                schema.contains(&example),
                "{workspace_type:?} schema must derive a concrete example for `{}`: {schema}",
                id_rule.label
            );
        }
    }
}

#[test]
fn retry_and_revision_prompts_render_parser_derived_schema() {
    let review = ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: "请补全结构。".to_string(),
        summary: "缺少 artifact schema".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    };

    for workspace_type in [
        WorkspaceType::Story,
        WorkspaceType::Design,
        WorkspaceType::WorkItem,
    ] {
        let (_tmp, store) = setup();
        let (event_tx, _event_rx) = mpsc::channel(8);
        let mut session = make_session(&format!("sess_schema_revision_{workspace_type:?}"));
        session.workspace_type = workspace_type.clone();
        let engine = WorkspaceEngine::new(store, event_tx, session);
        let spec = artifact_constraint_spec_for(&workspace_type);

        for (kind, prompt) in [
            (
                "retry",
                build_artifact_retry_prompt(
                    &workspace_type,
                    "上一轮输出",
                    &["缺少 heading".to_string()],
                ),
            ),
            ("delta revision", engine.build_revision_delta_prompt(&review)),
            (
                "full revision",
                engine.build_revision_full_prompt("# 上一版 artifact", &review),
            ),
        ] {
            assert!(
                prompt.contains("[artifact_schema_contract]"),
                "{workspace_type:?} {kind} prompt must repeat the parser-derived schema: {prompt}"
            );
            for label in spec
                .required_headings
                .iter()
                .map(|rule| rule.label)
                .chain(spec.required_id_patterns.iter().map(|rule| rule.label))
                .chain(spec.required_tokens.iter().map(|rule| rule.label))
            {
                assert!(
                    prompt.contains(label),
                    "{workspace_type:?} {kind} prompt must include parser label `{label}`: {prompt}"
                );
            }
        }
    }
}

#[test]
fn full_revision_prompt_does_not_repeat_schema_from_generation_context() {
    let review = ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: "请补全结构。".to_string(),
        summary: "缺少 artifact schema".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    };
    let (_tmp, store) = setup();
    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut session = make_session("sess_revision_schema_once");
    session.workspace_type = WorkspaceType::Story;
    session.messages.push(SessionMessage {
        id: "msg_generation_context".to_string(),
        role: "system".to_string(),
        content: format!(
            "[workflow_discipline]\n{}\n[output_schema]\n[artifact_schema_contract]\n来自正常 generation brief 的合同。",
            crate::product::cadence_skills::routing_reference::direct_cadence_routing_rules_reference()
        ),
        checkpoint_id: None,
        created_at: "2026-07-23T00:00:00Z".to_string(),
    });
    let engine = WorkspaceEngine::new(store, event_tx, session);

    let prompt = engine.build_revision_full_prompt("# 上一版 artifact", &review);

    assert_eq!(
        prompt.matches("[artifact_schema_contract]").count(),
        1,
        "full revision must reuse, not repeat, the schema already present in generation context: {prompt}"
    );
    assert_eq!(
        prompt.matches("[cadence_original_routing_rules]").count(),
        1,
        "full revision must reuse, not repeat, the direct routing reference already present in generation context: {prompt}"
    );
}

#[test]
fn reviewer_prompts_render_parser_derived_schema_gate() {
    for (workspace_type, artifact) in [
        (
            WorkspaceType::Story,
            complete_story_artifact("保留范围", "可以验收"),
        ),
        (
            WorkspaceType::Design,
            complete_design_artifact("保留边界", "公开接口保持稳定"),
        ),
        (
            WorkspaceType::WorkItem,
            complete_work_item_artifact("完成单项实现"),
        ),
    ] {
        let (_tmp, store) = setup();
        let (event_tx, _event_rx) = mpsc::channel(8);
        let mut session = make_session(&format!("sess_schema_reviewer_{workspace_type:?}"));
        session.workspace_type = workspace_type.clone();
        session.artifact = Some(artifact_payload(&artifact));
        let engine = WorkspaceEngine::new(store, event_tx, session);

        let prompt = engine.build_review_input().expect("review input").prompt;
        let spec = artifact_constraint_spec_for(&workspace_type);

        assert!(
            prompt.contains("[artifact_schema_review_gate]"),
            "{workspace_type:?} reviewer must receive the parser-derived schema gate: {prompt}"
        );
        assert!(
            prompt.contains("不得输出 `pass`"),
            "{workspace_type:?} reviewer must not pass parser-invalid artifacts: {prompt}"
        );
        for label in spec
            .required_headings
            .iter()
            .map(|rule| rule.label)
            .chain(spec.required_id_patterns.iter().map(|rule| rule.label))
            .chain(spec.required_tokens.iter().map(|rule| rule.label))
            .chain(spec.forbidden_headings.iter().map(|rule| rule.label))
            .chain(spec.forbidden_tokens.iter().map(|rule| rule.label))
        {
            assert!(
                prompt.contains(label),
                "{workspace_type:?} reviewer must receive parser label `{label}`: {prompt}"
            );
        }
    }
}
