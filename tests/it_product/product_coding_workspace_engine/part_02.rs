#[tokio::test]
async fn coding_tester_does_not_resume_coder_provider_session() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::ClaudeCode,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            ..create_input()
        })
        .expect("create attempt");
    let attempt = store
        .replace_attempt_provider_conversations(
            &attempt.id,
            vec![
                ProviderConversationRef {
                    role: ProviderConversationRole::Coder,
                    provider: ProviderName::ClaudeCode,
                    provider_session_id: "coder-session-1".to_string(),
                    updated_at: "2026-06-01T00:00:00Z".to_string(),
                    last_node_id: Some("coding-node-1".to_string()),
                },
                ProviderConversationRef {
                    role: ProviderConversationRole::Tester,
                    provider: ProviderName::ClaudeCode,
                    provider_session_id: "tester-session-1".to_string(),
                    updated_at: "2026-06-01T00:01:00Z".to_string(),
                    last_node_id: Some("testing-node-1".to_string()),
                },
            ],
        )
        .expect("persist provider conversations");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, mut rx) = mpsc::channel(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = SessionInputCapturingProvider::with_outputs(
        [
            r#"{"summary":"testing plan","steps":[{"id":"provider_check","title":"Provider check","intent":"verify provider session isolation","required":true,"tool":"provider_managed","risk_level":"low","command_or_tool_input":{},"evidence_expectation":"provider evidence","related_requirements":["REQ-TEST"],"related_design_constraints":["DEC-TEST"],"related_work_item_tasks":["TASK-TEST"]}]}"#,
            r#"{"step_results":[{"step_id":"provider_check","status":"passed","evidence_refs":["provider-session.log"],"provider_analysis":"session isolated"}]}"#,
        ],
        [None, Some("tester-session-2".to_string())],
    );

    let _report = engine
        .execute_testing_with_provider(
            &attempt,
            &provider,
            &CodingExecutionContext::default(),
            &[],
            TesterAgentOptions::default(),
        )
        .await
        .expect("testing provider run");

    let inputs = provider.inputs.lock().expect("inputs lock");
    assert_eq!(inputs.len(), 2);
    assert!(inputs[0].prompt.contains("Phase: plan_tests"));
    assert!(inputs[1].prompt.contains("Phase: execute_test_plan"));
    for input in inputs.iter() {
        assert_eq!(input.permission_mode, ProviderPermissionMode::Auto);
        assert_eq!(input.timeout_secs, 10_800);
        assert_eq!(input.resume_provider_session_id, None);
    }
    let updated = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("updated attempt");
    assert!(updated.provider_conversations.iter().any(|conversation| {
        conversation.role == ProviderConversationRole::Tester
            && conversation.provider == ProviderName::ClaudeCode
            && conversation.provider_session_id == "tester-session-2"
    }));
}

#[tokio::test]
async fn coding_tester_uses_role_permission_mode_auto() {
    let root = tempfile::tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    let mut role_config = store
        .get_role_provider_config_snapshot("project_0001", "issue_0001", &attempt.id)
        .expect("role config");
    role_config.set_permission_mode_for_role(
        &CodingProviderRole::Tester,
        CodingProviderPermissionMode::Auto,
    );
    store
        .update_role_provider_config_snapshot(
            "project_0001",
            "issue_0001",
            &attempt.id,
            role_config,
        )
        .expect("save role config");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");

    let (tx, mut rx) = mpsc::channel(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let provider = SessionInputCapturingProvider::with_outputs(
        [
            r#"{"summary":"unit","steps":[{"id":"unit","title":"Unit","intent":"verify unit","required":true,"tool":"provider_managed","risk_level":"low","command_or_tool_input":{},"evidence_expectation":"provider evidence","related_requirements":["REQ-UNIT"],"related_design_constraints":["DEC-UNIT"],"related_work_item_tasks":["TASK-UNIT"]}]}"#,
            r#"{"step_results":[{"step_id":"unit","status":"passed","evidence_refs":["unit.log"],"provider_analysis":"ok"}]}"#,
        ],
        [None, None],
    );

    engine
        .execute_testing_with_provider(
            &attempt,
            &provider,
            &CodingExecutionContext {
                work_item_markdown: Some("Work Item".to_string()),
                verification_commands: Vec::new(),
            },
            &[],
            TesterAgentOptions::default(),
        )
        .await
        .expect("testing");

    let inputs = provider.inputs.lock().expect("inputs");
    assert_eq!(inputs[0].permission_mode, ProviderPermissionMode::Auto);
    assert_eq!(inputs[1].permission_mode, ProviderPermissionMode::Auto);
}

#[tokio::test]
async fn execute_testing_binds_plan_report_and_chat_entries_to_tester_role_run() {
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
    let (tx, mut rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = ExecutePlanToolCallTesterProvider::new();

    let report = engine
        .execute_testing_with_provider(
            &attempt,
            &provider,
            &CodingExecutionContext::default(),
            &[],
            TesterAgentOptions::default(),
        )
        .await
        .expect("testing");

    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].role, CodingProviderRole::Tester);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Completed);
    assert_eq!(report.role_run_id.as_deref(), Some(runs[0].id.as_str()));
    assert_eq!(report.run_no, Some(1));
    let events = store
        .list_role_run_events("project_0001", "issue_0001", &attempt.id, &runs[0].id)
        .expect("role run events");
    let provider_prompts = events
        .iter()
        .filter(|event| event.event_type == CodingRoleRunEventType::ProviderPrompt)
        .collect::<Vec<_>>();
    assert_eq!(provider_prompts.len(), 2);
    assert!(provider_prompts.iter().any(|event| {
        event.payload["output_schema"] == "coding_workspace_test_plan_json"
            && event.payload["prompt"]
                .as_str()
                .is_some_and(|prompt| prompt.contains("Phase: plan_tests"))
    }));
    assert!(provider_prompts.iter().any(|event| {
        event.payload["output_schema"] == "coding_workspace_execute_test_plan_json"
            && event.payload["prompt"]
                .as_str()
                .is_some_and(|prompt| prompt.contains("Phase: execute_test_plan"))
    }));
    assert!(events.iter().any(|event| {
        event.event_type == CodingRoleRunEventType::ToolCall
            && event.payload["id"] == "execute_tool_0001"
            && event.payload["tool_name"] == "run_command"
    }));
    assert!(events.iter().any(|event| {
        event.event_type == CodingRoleRunEventType::ToolResult
            && event.payload["tool_use_id"] == "execute_tool_0001"
            && event.payload["is_error"] == false
    }));
    assert!(
        events
            .iter()
            .any(|event| event.event_type == CodingRoleRunEventType::MessageComplete)
    );

    let plans = store
        .list_test_plans("project_0001", "issue_0001", &attempt.id)
        .expect("plans");
    assert_eq!(plans[0].role_run_id.as_deref(), Some(runs[0].id.as_str()));
    assert_eq!(plans[0].run_no, Some(1));

    let mut saw_plan_entry = false;
    let mut saw_result_entry = false;
    while let Ok(message) = rx.try_recv() {
        if let CodingWsOutMessage::CodingChatEntryCreated { entry } = message {
            let metadata = entry.metadata.unwrap_or_default();
            let content = entry.content.unwrap_or_default();
            if metadata.get("role_run_id").and_then(|value| value.as_str())
                == Some(runs[0].id.as_str())
                && metadata.get("phase").and_then(|value| value.as_str()) == Some("test_plan")
            {
                saw_plan_entry = true;
                assert!(content.contains("unit plan"));
            }
            if metadata.get("role_run_id").and_then(|value| value.as_str())
                == Some(runs[0].id.as_str())
                && metadata.get("phase").and_then(|value| value.as_str()) == Some("testing_result")
            {
                saw_result_entry = true;
                assert!(content.contains("passed"));
            }
        }
    }
    assert!(saw_plan_entry);
    assert!(saw_result_entry);
}

#[tokio::test]
async fn execute_testing_blocks_when_provider_completes_before_choice_response() {
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
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = ExecutePlanChoiceThenCompletedTesterProvider::default();

    let error = engine
        .execute_testing_with_provider(
            &attempt,
            &provider,
            &CodingExecutionContext::default(),
            &[],
            TesterAgentOptions::default(),
        )
        .await
        .expect_err("provider cannot complete execute_test_plan with unresolved choice");

    assert_eq!(
        error.to_string(),
        "coding_provider_stream_failed: provider_choice_unresolved"
    );
    assert_eq!(
        store
            .get_attempt("project_0001", "issue_0001", &attempt.id)
            .expect("attempt")
            .status,
        CodingAttemptStatus::WaitingForHuman
    );
    assert_eq!(
        store
            .list_open_choice_gates("project_0001", "issue_0001", &attempt.id)
            .expect("open choice gates")
            .len(),
        1
    );
    let events = drain_events(&mut rx);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            CodingWsOutMessage::CodingChoiceRequest {
                id,
                source,
                ..
            } if id == "choice_0001" && source == "ask_user_question"
        )
    }));
}

#[tokio::test]
async fn tester_plan_timeout_blocks_with_retry_test_plan_gate() {
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
    let (tx, _rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let result = tokio::time::timeout(
        Duration::from_secs(2),
        engine.execute_testing_with_provider(
            &attempt,
            &HangingPlanTesterProvider,
            &CodingExecutionContext::default(),
            &[],
            TesterAgentOptions {
                timeout: Duration::from_millis(20),
                failure_limit: 3,
            },
        ),
    )
    .await
    .expect("engine should return before outer timeout");
    let report = result.expect("timeout becomes blocked report");

    assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
    assert!(
        report
            .context_warnings
            .contains(&"plan_tests_timeout".to_string())
    );
    let gates = store
        .list_open_blocked_gates("project_0001", "issue_0001", &attempt.id)
        .expect("gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].reason_code.as_deref(), Some("plan_tests_timeout"));
    assert!(
        gates[0]
            .available_actions
            .iter()
            .any(|action| action.action_id == "retry_test_plan")
    );
}

#[tokio::test]
async fn tester_plan_start_timeout_blocks_with_retry_test_plan_gate() {
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
    let (tx, _rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let result = tokio::time::timeout(
        Duration::from_secs(2),
        engine.execute_testing_with_provider(
            &attempt,
            &NeverStartingTesterProvider,
            &CodingExecutionContext::default(),
            &[],
            TesterAgentOptions {
                timeout: Duration::from_millis(20),
                failure_limit: 3,
            },
        ),
    )
    .await
    .expect("engine should return before outer timeout");
    let report = result.expect("start timeout becomes blocked report");

    assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
    assert!(
        report
            .context_warnings
            .contains(&"plan_tests_timeout".to_string())
    );
    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Blocked);
    assert_eq!(runs[0].reason_code.as_deref(), Some("plan_tests_timeout"));
    let events = store
        .list_role_run_events("project_0001", "issue_0001", &attempt.id, &runs[0].id)
        .expect("role run events");
    assert!(
        events
            .iter()
            .any(|event| event.event_type == CodingRoleRunEventType::ProviderPrompt)
    );
    assert!(
        events
            .iter()
            .any(|event| event.event_type == CodingRoleRunEventType::Timeout)
    );
    let gates = store
        .list_open_blocked_gates("project_0001", "issue_0001", &attempt.id)
        .expect("gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].reason_code.as_deref(), Some("plan_tests_timeout"));
    assert!(
        gates[0]
            .available_actions
            .iter()
            .any(|action| action.action_id == "retry_test_plan")
    );
}

#[tokio::test]
async fn tester_execute_plan_start_timeout_blocks_with_retry_gate() {
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
    let (tx, _rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let result = tokio::time::timeout(
        Duration::from_secs(2),
        engine.execute_testing_with_provider(
            &attempt,
            &HangingExecutePlanStartTesterProvider::default(),
            &CodingExecutionContext::default(),
            &[],
            TesterAgentOptions {
                timeout: Duration::from_millis(20),
                failure_limit: 3,
            },
        ),
    )
    .await
    .expect("engine should return before outer timeout");
    let report = result.expect("execute start timeout becomes blocked report");

    assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
    assert!(
        report
            .context_warnings
            .contains(&"execute_test_plan_timeout".to_string())
    );
    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Blocked);
    assert_eq!(
        runs[0].reason_code.as_deref(),
        Some("execute_test_plan_timeout")
    );
    let events = store
        .list_role_run_events("project_0001", "issue_0001", &attempt.id, &runs[0].id)
        .expect("role run events");
    assert!(events.iter().any(|event| {
        event.event_type == CodingRoleRunEventType::ProviderPrompt
            && event.payload["output_schema"] == "coding_workspace_execute_test_plan_json"
            && event.payload["prompt"]
                .as_str()
                .is_some_and(|prompt| prompt.contains("Phase: execute_test_plan"))
    }));
    assert!(events.iter().any(|event| {
        event.event_type == CodingRoleRunEventType::Timeout
            && event.payload["phase"] == "execute_test_plan_start"
            && event.payload["reason_code"] == "execute_test_plan_timeout"
    }));
    let gates = store
        .list_open_blocked_gates("project_0001", "issue_0001", &attempt.id)
        .expect("gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("execute_test_plan_timeout")
    );
    assert!(
        gates[0]
            .available_actions
            .iter()
            .any(|action| action.action_id == "retry_test_plan")
    );
}

#[tokio::test]
async fn blocked_testing_gate_reason_overrides_report_warning_for_role_run() {
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
    let (tx, _rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let report = tokio::time::timeout(
        Duration::from_secs(2),
        engine.execute_testing_with_provider(
            &attempt,
            &HangingExecutePlanStartTesterProvider::with_plan_warning("timeout budget risk"),
            &CodingExecutionContext::default(),
            &[],
            TesterAgentOptions {
                timeout: Duration::from_millis(20),
                failure_limit: 3,
            },
        ),
    )
    .await
    .expect("engine should return before outer timeout")
    .expect("execute start timeout becomes blocked report");

    assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
    assert!(
        report
            .context_warnings
            .contains(&"timeout budget risk".to_string())
    );
    assert!(
        report
            .context_warnings
            .contains(&"execute_test_plan_timeout".to_string())
    );
    let gates = store
        .list_open_blocked_gates("project_0001", "issue_0001", &attempt.id)
        .expect("gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("execute_test_plan_timeout")
    );
    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0].reason_code.as_deref(),
        Some("execute_test_plan_timeout")
    );
}

