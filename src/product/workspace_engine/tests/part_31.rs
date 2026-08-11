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
            prompt.contains("[cadence_project_rules]"),
            "initial {workspace_type:?} author prompt must include the direct routing reference: {prompt}"
        );
        assert_eq!(
            prompt.matches("[cadence_project_rules]").count(),
            1,
            "initial {workspace_type:?} author fallback must include the direct routing reference exactly once: {prompt}"
        );
        assert!(
            prompt.contains("AGENTS.md") && prompt.contains("CLAUDE.md"),
            "initial {workspace_type:?} author prompt must name both project rule files: {prompt}"
        );
        assert!(
            !prompt.contains(&["Cadence-", "skills/"].concat()),
            "{prompt}"
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
            prompt.matches("[cadence_project_rules]").count(),
            1,
            "initial {workspace_type:?} author prompt must reuse, not repeat, the system routing reference: {prompt}"
        );
        assert!(prompt.contains("[cadence_project_rules]"), "{prompt}");
        assert!(prompt.contains("AGENTS.md") && prompt.contains("CLAUDE.md"), "{prompt}");
        assert!(
            !prompt.contains(&["Cadence-", "skills/"].concat()),
            "{prompt}"
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
fn story_schema_contract_exposes_open_item_resolution_protocol() {
    let schema = author_artifact_schema_contract_for(&WorkspaceType::Story)
        .expect("Story must have a schema contract");
    assert!(
        schema.contains("已通过 AskUserQuestion 确认"),
        "Story schema contract must teach the resolved-cue protocol for 待确认项: {schema}"
    );
    assert!(
        schema.contains("无待确认项"),
        "Story schema contract must teach the empty marker for 待确认项: {schema}"
    );
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
        prompt.matches("[cadence_project_rules]").count(),
        1,
        "full revision must reuse, not repeat, the direct routing reference already present in generation context: {prompt}"
    );
    assert!(prompt.contains("[cadence_project_rules]"), "{prompt}");
    assert!(prompt.contains("AGENTS.md") && prompt.contains("CLAUDE.md"), "{prompt}");
    assert!(
        !prompt.contains(&["Cadence-", "skills/"].concat()),
        "{prompt}"
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

#[test]
fn author_streaming_input_uses_session_permission_mode() {
    let (_tmp, store) = setup();
    let (tx, _rx) = mpsc::channel(64);
    let mut session = make_session("sess_author_permission_mode");
    session.permission_modes.author = ProviderPermissionMode::Auto;
    let engine = WorkspaceEngine::new(store, tx, session);

    let input = engine
        .build_streaming_input("start", AuthorPromptMode::FullConversation)
        .expect("author input");

    assert_eq!(input.permission_mode, ProviderPermissionMode::Auto);
}

#[tokio::test]
async fn start_generation_locks_selected_modes_into_store() {
    let root = tempfile::tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    let record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "p1".to_string(),
            issue_id: "i1".to_string(),
            entity_id: "e1".to_string(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
        })
        .expect("create session");
    let session_id = record.id.clone();
    let checkpoint_store = Arc::new(CheckpointStore::new(
        paths.issue_lifecycle_root("p1", "i1"),
    ));
    let (tx, _rx) = mpsc::channel(64);
    let mut engine = WorkspaceEngine::new_persistent(
        checkpoint_store,
        lifecycle.clone(),
        tx,
        WorkspaceSession::from_record(record),
    );
    let wire = ProviderConfigSnapshot {
        author: ProviderName::ClaudeCode,
        reviewer: Some(ProviderName::Codex),
        review_rounds: 1,
        permission_modes: crate::product::models::WorkspaceRolePermissionModes {
            author: ProviderPermissionMode::Supervised,
            reviewer: ProviderPermissionMode::Auto,
        },
    };

    engine.start_generation(wire, true).await.expect("start generation");

    let reread = lifecycle.get_workspace_session(&session_id).expect("reload");
    assert_eq!(reread.permission_modes.author, ProviderPermissionMode::Supervised);
    assert_eq!(reread.permission_modes.reviewer, ProviderPermissionMode::Auto);
}

#[tokio::test]
async fn start_generation_normalizes_pi_role_to_auto_and_keeps_disabled_reviewer_mode() {
    let root = tempfile::tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    let record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "p1".to_string(),
            issue_id: "i1".to_string(),
            entity_id: "e1".to_string(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::Pi,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
        })
        .expect("create session");
    let session_id = record.id.clone();
    let checkpoint_store = Arc::new(CheckpointStore::new(
        paths.issue_lifecycle_root("p1", "i1"),
    ));
    let (tx, _rx) = mpsc::channel(64);
    let mut engine = WorkspaceEngine::new_persistent(
        checkpoint_store,
        lifecycle.clone(),
        tx,
        WorkspaceSession::from_record(record),
    );
    let wire = ProviderConfigSnapshot {
        author: ProviderName::Pi,
        reviewer: None,
        review_rounds: 0,
        permission_modes: crate::product::models::WorkspaceRolePermissionModes {
            author: ProviderPermissionMode::Supervised,
            reviewer: ProviderPermissionMode::Supervised,
        },
    };

    engine.start_generation(wire, false).await.expect("start generation");

    let reread = lifecycle.get_workspace_session(&session_id).expect("reload");
    assert_eq!(reread.permission_modes.author, ProviderPermissionMode::Auto);
    assert_eq!(
        reread.permission_modes.reviewer,
        ProviderPermissionMode::Supervised,
        "disabled reviewer retains its selected future-run mode"
    );
}

struct CountingProvider {
    starts: Arc<std::sync::atomic::AtomicUsize>,
    seen: Arc<Mutex<Vec<(ProviderType, ProviderPermissionMode)>>>,
    fail_on_start: bool,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for CountingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        use std::sync::atomic::Ordering;

        self.starts.fetch_add(1, Ordering::SeqCst);
        self.seen
            .lock()
            .unwrap()
            .push((input.provider_type.clone(), input.permission_mode.clone()));
        if self.fail_on_start {
            return Err(ProviderAdapterError::execution_failed(
                None,
                String::new(),
                "pi start failed",
                0,
            ));
        }
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let output = complete_story_artifact("生成候选草稿。", "候选草稿可进入审核。");
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: output.clone(),
                })
                .await;
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::plain(
                    output,
                    Some("sess-1".to_string()),
                )))
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
            "unused",
            0,
        ))
    }
}

#[tokio::test]
async fn author_run_with_pi_uses_pi_provider_in_auto_mode() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let (_tmp, store) = setup();
    let (tx, _rx) = mpsc::channel(64);
    let mut session = make_session("sess_pi");
    session.author_provider = ProviderName::Pi;
    session.permission_modes.author = ProviderPermissionMode::Auto;
    let mut engine = WorkspaceEngine::new(store, tx, session);

    let pi_starts = Arc::new(AtomicUsize::new(0));
    let pi_seen = Arc::new(Mutex::new(Vec::new()));
    let provider = CountingProvider {
        starts: pi_starts.clone(),
        seen: pi_seen.clone(),
        fail_on_start: false,
    };

    engine
        .handle_user_message("start".to_string(), Arc::new(provider), empty_provider_commands())
        .await;

    assert_eq!(pi_starts.load(Ordering::SeqCst), 1, "Pi 应被调用一次");
    let seen = pi_seen.lock().unwrap();
    assert_eq!(seen[0].0, ProviderType::Pi);
    assert_eq!(seen[0].1, ProviderPermissionMode::Auto, "Pi 仅 Auto");
}

#[tokio::test]
async fn pi_author_runs_from_story_design_and_work_item_entries_in_auto_mode() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    for workspace_type in [
        WorkspaceType::Story,
        WorkspaceType::Design,
        WorkspaceType::WorkItem,
    ] {
        let (_tmp, store) = setup();
        let (tx, _rx) = mpsc::channel(64);
        let mut session = make_session(&format!("sess_pi_{workspace_type:?}"));
        session.workspace_type = workspace_type.clone();
        session.author_provider = ProviderName::Pi;
        session.permission_modes.author = ProviderPermissionMode::Supervised;
        let mut engine = WorkspaceEngine::new(store, tx, session);

        let pi_starts = Arc::new(AtomicUsize::new(0));
        let pi_seen = Arc::new(Mutex::new(Vec::new()));
        let provider = CountingProvider {
            starts: pi_starts.clone(),
            seen: pi_seen.clone(),
            fail_on_start: false,
        };

        engine
            .handle_user_message("start".to_string(), Arc::new(provider), empty_provider_commands())
            .await;

        assert_eq!(
            pi_starts.load(Ordering::SeqCst),
            1,
            "Pi must start once from the {workspace_type:?} author entry"
        );
        let seen = pi_seen.lock().unwrap();
        assert_eq!(
            seen.as_slice(),
            &[(ProviderType::Pi, ProviderPermissionMode::Auto)],
            "the {workspace_type:?} author entry must select Pi and normalize it to Auto"
        );
    }
}

#[tokio::test]
async fn kimi_author_does_not_retry_missing_artifact() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let (_tmp, store) = setup();
    let (tx, _rx) = mpsc::channel(64);
    let mut session = make_session("sess_kimi_no_artifact_retry");
    session.author_provider = ProviderName::KimiCode;
    session.reviewer_provider = None;
    let mut engine = WorkspaceEngine::new(store, tx, session);
    let starts = Arc::new(AtomicUsize::new(0));
    let provider = KimiIncompleteArtifactProvider {
        starts: starts.clone(),
    };

    engine
        .handle_user_message("start".to_string(), Arc::new(provider), empty_provider_commands())
        .await;

    assert_eq!(starts.load(Ordering::SeqCst), 1);
    assert_eq!(engine.session().stage, WorkspaceStage::PrepareContext);
}

struct KimiIncompleteArtifactProvider {
    starts: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for KimiIncompleteArtifactProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let output = "Kimi returned an incomplete artifact.".to_string();
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::plain(output, None)))
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

#[tokio::test]
async fn pi_start_failure_does_not_retry_selected_provider() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let (_tmp, store) = setup();
    let (tx, _rx) = mpsc::channel(64);
    let mut session = make_session("sess_pi_fail");
    session.author_provider = ProviderName::Pi;
    let mut engine = WorkspaceEngine::new(store, tx, session);

    let pi_starts = Arc::new(AtomicUsize::new(0));
    let provider = CountingProvider {
        starts: pi_starts.clone(),
        seen: Arc::new(Mutex::new(Vec::new())),
        fail_on_start: true,
    };

    engine
        .handle_user_message("start".to_string(), Arc::new(provider), empty_provider_commands())
        .await;

    assert_eq!(pi_starts.load(Ordering::SeqCst), 1, "Pi 只启动一次，不重试");
    assert_eq!(engine.session().stage, WorkspaceStage::PrepareContext);
}
