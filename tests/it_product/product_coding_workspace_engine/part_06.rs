#[tokio::test]
async fn execute_code_review_blocked_keeps_attempt_running_for_analyst() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\nreviewed\n").expect("modify file");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = InputCapturingProvider {
        input: Arc::new(Mutex::new(None)),
        output: r#"{
            "verdict": "blocked",
            "summary": "缺少人工测试账号，无法完成 review",
            "findings": []
        }"#
        .to_string(),
    };

    let report = engine
        .execute_code_review(&attempt, &provider)
        .await
        .expect("execute code review");

    assert_eq!(report.verdict, ReviewVerdict::Blocked);
    assert_eq!(report.summary, "缺少人工测试账号，无法完成 review");
    let updated = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("updated attempt");
    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::CodeReview);
    let gates = store
        .list_open_blocked_gates("project_0001", "issue_0001", &attempt.id)
        .expect("open blocked gates");
    assert!(gates.is_empty());
}

#[tokio::test]
async fn code_review_provider_start_failure_marks_attempt_blocked_and_node_failed() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\nreviewed\n").expect("modify file");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = StartFailingProvider;

    let error = engine
        .execute_code_review(&attempt, &provider)
        .await
        .expect_err("provider start should fail");

    assert!(error.to_string().contains("provider failed to start"));
    let updated = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("updated attempt");
    assert_eq!(updated.status, CodingAttemptStatus::Failed);
    assert_eq!(updated.stage, CodingExecutionStage::CodeReview);
    let nodes = store
        .get_timeline_nodes("project_0001", "issue_0001", &attempt.id)
        .expect("timeline nodes");
    let node = nodes.last().expect("code review node");
    assert_eq!(node.stage, CodingExecutionStage::CodeReview);
    assert_eq!(node.status, CodingTimelineNodeStatus::Failed);
    assert_eq!(node.summary.as_deref(), Some("provider failed to start"));
    assert!(node.completed_at.is_some());
}

#[tokio::test]
async fn execute_code_review_prompt_includes_diff_work_item_rules_and_role_provider() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\nstairs implementation\n").expect("modify file");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    seed_work_item_markdown(
        &app_paths,
        "实现爬楼梯问题：给定 n 阶楼梯，每次可以爬 1 或 2 阶。",
    );
    let store = CodingAttemptStore::new(app_paths);
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_role_provider_config_snapshot(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingRoleProviderConfigSnapshot {
                coder: ProviderName::Fake,
                tester: ProviderName::Fake,
                analyst: ProviderName::Fake,
                code_reviewer: ProviderName::Codex,
                internal_reviewer: ProviderName::Fake,
                review_rounds: 1,
                permission_modes: CodingRolePermissionModes::default(),
            },
        )
        .expect("set role provider snapshot");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let captured_input = Arc::new(Mutex::new(None));
    let provider = InputCapturingProvider {
        input: captured_input.clone(),
        output: r#"{"verdict":"approve","summary":"review ok","findings":[]}"#.to_string(),
    };

    engine
        .execute_code_review(&attempt, &provider)
        .await
        .expect("execute code review");

    let input = captured_input
        .lock()
        .expect("input lock")
        .clone()
        .expect("captured input");
    assert_eq!(input.provider_type, ProviderType::Codex);
    assert_eq!(input.role, AdapterRole::Reviewer);
    assert_eq!(input.output_schema, "coding_workspace_code_review_json");
    assert!(input.prompt.contains("CodeReviewer"));
    assert!(input.prompt.contains("git diff"));
    assert!(input.prompt.contains("+stairs implementation"));
    assert!(input.prompt.contains("实现爬楼梯问题"));
    assert!(input.prompt.contains("代码规范"));
}

#[tokio::test]
async fn execute_rework_needs_fix_uses_analyst_prompt_consumes_notes_and_routes_to_coding() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree.clone()),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("testing stage");
    let old_note = store
        .create_context_note(&attempt.id, "不要出现在 prompt".to_string())
        .expect("old context note");
    store
        .mark_context_notes_consumed("project_0001", "issue_0001", &attempt.id, &[old_note.id], 1)
        .expect("consume old note");
    let new_note = store
        .create_context_note(&attempt.id, "请补充 n=10 的测试".to_string())
        .expect("new context note");
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let captured_prompt = Arc::new(Mutex::new(None));
    let provider = AnalystStreamingProvider {
        prompt: captured_prompt.clone(),
        output: r#"{"verdict":"needs_fix","summary":"测试仍失败","fix_hints":["补充 climb_stairs 动态规划实现"]}"#.to_string(),
    };

    let updated = engine
        .execute_rework(&attempt, "测试失败: unit failed", &provider)
        .await
        .expect("execute rework");

    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::Coding);
    assert_eq!(updated.rework_count, 1);
    assert!(!worktree.join("reworked.txt").exists());
    let prompt = captured_prompt
        .lock()
        .expect("prompt lock")
        .clone()
        .expect("captured prompt");
    assert!(prompt.contains("Rework 分析官"));
    assert!(prompt.contains("只做分析和路由决策"));
    assert!(prompt.contains("不要修改代码"));
    assert!(prompt.contains("测试失败: unit failed"));
    assert!(prompt.contains("请补充 n=10 的测试"));
    assert!(!prompt.contains("不要出现在 prompt"));
    let notes = store
        .list_context_notes("project_0001", "issue_0001", &attempt.id)
        .expect("notes");
    assert_eq!(notes[1].id, new_note.id);
    assert_eq!(notes[1].consumed_by_rework_round, Some(1));
    let nodes = store
        .get_timeline_nodes("project_0001", "issue_0001", &attempt.id)
        .expect("timeline nodes");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].stage, CodingExecutionStage::Rework);
    assert_eq!(nodes[0].title, "分析官判定 #1");
    assert_eq!(nodes[0].status, CodingTimelineNodeStatus::Completed);
    assert_eq!(nodes[0].summary.as_deref(), Some("NeedsFix: 测试仍失败"));
    let instruction = store
        .latest_unconsumed_rework_instruction("project_0001", "issue_0001", &attempt.id)
        .expect("latest rework instruction")
        .expect("rework instruction");
    assert_eq!(instruction.source_stage, CodingExecutionStage::Testing);
    assert_eq!(instruction.rework_round, 1);
    assert_eq!(instruction.summary, "测试仍失败");
    assert_eq!(
        instruction.fix_hints,
        vec!["补充 climb_stairs 动态规划实现".to_string()]
    );

    let events = drain_events(&mut rx);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            CodingWsOutMessage::CodingTimelineNodeCreated { node }
                if node.id == "coding_node_0001"
                    && node.stage == CodingExecutionStage::Rework
                    && node.title == "分析官判定 #1"
                    && node.status == CodingTimelineNodeStatus::Running
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            CodingWsOutMessage::CodingMessageComplete {
                node_id: Some(node_id)
            } if node_id == "coding_node_0001"
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            CodingWsOutMessage::CodingChatEntryCreated { entry }
                if matches!(
                    &entry.entry_type,
                    CodingEntryType::AnalystVerdict {
                        verdict: AnalystVerdict::NeedsFix
                    }
                ) && entry.content.as_deref() == Some("测试仍失败")
                    && entry
                        .metadata
                        .as_ref()
                        .and_then(|metadata| metadata.get("fix_hints"))
                        .and_then(|value| value.as_array())
                        .and_then(|items| items.first())
                        .and_then(|value| value.as_str())
                        == Some("补充 climb_stairs 动态规划实现")
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            CodingWsOutMessage::CodingTimelineNodeUpdated {
                node_id,
                status: CodingTimelineNodeStatus::Completed,
                summary: Some(summary),
                completed_at: Some(_),
            } if node_id == "coding_node_0001" && summary == "NeedsFix: 测试仍失败"
        )
    }));
}

#[tokio::test]
async fn execute_rework_persists_structured_analyst_decision() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("testing stage");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = AnalystStreamingProvider {
        prompt: Arc::new(Mutex::new(None)),
        output: r#"{
            "verdict":"needs_fix",
            "next_stage":"coding",
            "reason":"required 测试步骤被跳过",
            "evidence_refs":["testing_report_0001.json"],
            "raw_provider_output_refs":["provider-raw/testing/execute_test_plan_0001.txt"],
            "rework_instructions":{
                "summary":"补齐 required 测试覆盖",
                "required_changes":["补充 B6 浏览器测试"],
                "verification_expectations":["B6 不再出现在 skipped_required_steps"]
            },
            "human_gate":null
        }"#
        .to_string(),
    };

    let updated = engine
        .execute_rework(&attempt, "testing blocked", &provider)
        .await
        .expect("execute rework");

    assert_eq!(updated.stage, CodingExecutionStage::Coding);
    let decision = store
        .latest_analyst_decision("project_0001", "issue_0001", &attempt.id)
        .expect("latest decision")
        .expect("persisted decision");
    assert_eq!(decision.id, "analyst_decision_0001");
    assert_eq!(decision.source_stage, CodingExecutionStage::Testing);
    assert_eq!(decision.rework_round, 1);
    assert_eq!(decision.verdict, AnalystDecisionVerdict::NeedsFix);
    assert_eq!(decision.next_stage, AnalystDecisionNextStage::Coding);
    assert_eq!(decision.reason, "required 测试步骤被跳过");
    assert_eq!(
        decision.evidence_refs,
        vec!["testing_report_0001.json".to_string()]
    );
    assert_eq!(
        decision.raw_provider_output_refs,
        vec!["provider-raw/testing/execute_test_plan_0001.txt".to_string()]
    );
    let rework = decision.rework_instructions.expect("rework instructions");
    assert_eq!(rework.summary, "补齐 required 测试覆盖");
    assert_eq!(
        rework.required_changes,
        vec!["补充 B6 浏览器测试".to_string()]
    );
    assert_eq!(
        rework.verification_expectations,
        vec!["B6 不再出现在 skipped_required_steps".to_string()]
    );
    assert_eq!(decision.parse_error, None);
}

#[tokio::test]
async fn execute_rework_normalizes_string_rework_instructions() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("testing stage");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = AnalystStreamingProvider {
        prompt: Arc::new(Mutex::new(None)),
        output: r#"{
            "verdict":"needs_fix",
            "next_stage":"coding",
            "reason":"仍有静默 Codex 默认",
            "evidence_refs":["testing_report_0001.steps.step_003_search_anchors"],
            "raw_provider_output_refs":["provider-raw/testing/execute_test_plan_0001.txt"],
            "rework_instructions":"修复 src/web/handlers.rs 和 src/web/runtime.rs 中残留的 codex 默认，并补充回归测试。",
            "human_gate":null
        }"#
        .to_string(),
    };

    let updated = engine
        .execute_rework(&attempt, "testing blocked", &provider)
        .await
        .expect("execute rework");

    assert_eq!(updated.stage, CodingExecutionStage::Coding);
    let decision = store
        .latest_analyst_decision("project_0001", "issue_0001", &attempt.id)
        .expect("latest decision")
        .expect("persisted decision");
    assert_eq!(decision.verdict, AnalystDecisionVerdict::NeedsFix);
    assert_eq!(decision.next_stage, AnalystDecisionNextStage::Coding);
    assert_eq!(decision.parse_error, None);
    let rework = decision.rework_instructions.expect("rework instructions");
    assert_eq!(
        rework.summary,
        "修复 src/web/handlers.rs 和 src/web/runtime.rs 中残留的 codex 默认，并补充回归测试。"
    );
    assert_eq!(
        rework.required_changes,
        vec![
            "修复 src/web/handlers.rs 和 src/web/runtime.rs 中残留的 codex 默认，并补充回归测试。"
                .to_string()
        ]
    );
    assert!(rework.verification_expectations.is_empty());
}

#[tokio::test]
async fn execute_rework_persists_legacy_analyst_verdict_as_decision() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = AnalystStreamingProvider {
        prompt: Arc::new(Mutex::new(None)),
        output: r#"{"verdict":"no_issue","summary":"审查通过"}"#.to_string(),
    };

    let updated = engine
        .execute_rework(&attempt, "code review approve", &provider)
        .await
        .expect("execute rework");

    assert_eq!(updated.stage, CodingExecutionStage::ReviewRequest);
    let decision = store
        .latest_analyst_decision("project_0001", "issue_0001", &attempt.id)
        .expect("latest decision")
        .expect("persisted decision");
    assert_eq!(decision.verdict, AnalystDecisionVerdict::Proceed);
    assert_eq!(decision.next_stage, AnalystDecisionNextStage::ReviewRequest);
    assert_eq!(decision.reason, "审查通过");
    assert_eq!(decision.rework_instructions, None);
    assert_eq!(decision.human_gate, None);
}

#[tokio::test]
async fn execute_rework_consumes_next_stage_testing_without_coding_rework() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("testing stage");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = AnalystStreamingProvider {
        prompt: Arc::new(Mutex::new(None)),
        output: r#"{
            "verdict":"rerun_testing",
            "next_stage":"testing",
            "reason":"Tester evidence is incomplete; rerun required browser steps",
            "evidence_refs":["testing_report_0001.json"]
        }"#
        .to_string(),
    };

    let updated = engine
        .execute_rework(&attempt, "testing evidence incomplete", &provider)
        .await
        .expect("execute rework");

    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::Testing);
    assert_eq!(updated.rework_count, 0);
    assert!(
        store
            .latest_unconsumed_rework_instruction("project_0001", "issue_0001", &attempt.id)
            .expect("latest rework instruction")
            .is_none()
    );
    let decision = store
        .latest_analyst_decision("project_0001", "issue_0001", &attempt.id)
        .expect("latest decision")
        .expect("persisted decision");
    assert_eq!(decision.verdict, AnalystDecisionVerdict::RerunTesting);
    assert_eq!(decision.next_stage, AnalystDecisionNextStage::Testing);
}

#[tokio::test]
async fn execute_rework_consumes_next_stage_code_review() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = AnalystStreamingProvider {
        prompt: Arc::new(Mutex::new(None)),
        output: r#"{
            "verdict":"proceed",
            "next_stage":"code_review",
            "reason":"Run CodeReviewer again after context-only clarification"
        }"#
        .to_string(),
    };

    let updated = engine
        .execute_rework(&attempt, "review clarification", &provider)
        .await
        .expect("execute rework");

    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::CodeReview);
    let decision = store
        .latest_analyst_decision("project_0001", "issue_0001", &attempt.id)
        .expect("latest decision")
        .expect("persisted decision");
    assert_eq!(decision.verdict, AnalystDecisionVerdict::Proceed);
    assert_eq!(decision.next_stage, AnalystDecisionNextStage::CodeReview);
}

