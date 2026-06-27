#[tokio::test]
async fn retry_test_plan_supersedes_latest_testing_role_run_and_resumes_testing() {
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let worktree = root.path().join("worktree");
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    fs::create_dir_all(attempt.worktree_path.as_ref().expect("worktree")).expect("worktree dir");
    let attempt = store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let attempt = store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("testing");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )
        .expect("blocked");
    let first_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0003".to_string()),
        )
        .expect("first run");
    store
        .update_role_run_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &first_run.id,
            CodingRoleRunStatus::Blocked,
            Some("plan_tests_timeout".to_string()),
        )
        .expect("blocked run");
    let gate = store
        .create_blocked_gate(CreateBlockedGateInput {
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::Testing,
            node_id: Some("coding_node_0003".to_string()),
            role: Some(CodingProviderRole::Tester),
            title: "Testing blocked".to_string(),
            description: "Tester plan timeout".to_string(),
            reason_code: Some("plan_tests_timeout".to_string()),
            evidence_refs: vec![],
            raw_provider_output_ref: None,
            available_actions: vec![
                CodingGateAction {
                    action_id: "retry_test_plan".to_string(),
                    label: "重新执行 Tester".to_string(),
                    action_type: CodingGateActionType::RetryTestPlan,
                },
                CodingGateAction {
                    action_id: "send_raw_output_to_analyst".to_string(),
                    label: "发送给 Analyst 决策".to_string(),
                    action_type: CodingGateActionType::SendRawOutputToAnalyst,
                },
                CodingGateAction {
                    action_id: "abort".to_string(),
                    label: "终止".to_string(),
                    action_type: CodingGateActionType::Abort,
                },
            ],
        })
        .expect("gate");
    let (tx, mut rx) = mpsc::channel(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let updated = engine
        .handle_blocked_gate_response(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &gate.gate_id,
            "retry_test_plan",
            None,
        )
        .await
        .expect("gate response");

    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::Testing);
    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("runs");
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Superseded);
    assert_eq!(runs[1].trigger, CodingRoleRunTrigger::RetryTestPlan);
    assert_eq!(runs[1].run_no, 2);
    assert_eq!(runs[1].node_id, None);

    let provider = SessionInputCapturingProvider::with_outputs(
        [
            r#"{"summary":"retry plan","steps":[{"id":"unit","title":"Unit","intent":"run unit checks","required":true,"tool":"provider_managed","risk_level":"low","command_or_tool_input":{},"evidence_expectation":"unit evidence","related_requirements":["REQ-UNIT"],"related_design_constraints":["DEC-UNIT"],"related_work_item_tasks":["TASK-UNIT"]}]}"#,
            r#"{"step_results":[{"step_id":"unit","status":"passed","evidence_refs":["unit.log"],"provider_analysis":"ok"}]}"#,
        ],
        [None, None],
    );
    let report = engine
        .execute_testing_with_provider(
            &updated,
            &provider,
            &CodingExecutionContext::default(),
            &[],
            TesterAgentOptions::default(),
        )
        .await
        .expect("rerun testing");

    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("runs after rerun");
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[1].status, CodingRoleRunStatus::Completed);
    assert_eq!(report.role_run_id.as_deref(), Some(runs[1].id.as_str()));
    assert!(runs[1].node_id.is_some());
}

#[tokio::test]
async fn retry_test_plan_prompt_includes_previous_role_run_diagnostic() {
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
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )
        .expect("blocked");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("testing");

    let first_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0001".to_string()),
        )
        .expect("first run");
    store
        .append_role_run_event(
            &attempt,
            &first_run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Task update",
                "status": "running",
                "detail": "No tasks found"
            }),
        )
        .expect("event");
    store
        .append_role_run_event(
            &attempt,
            &first_run,
            CodingRoleRunEventType::Timeout,
            serde_json::json!({
                "reason_code": "plan_tests_timeout",
                "message": "timed out"
            }),
        )
        .expect("timeout");
    store
        .update_role_run_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &first_run.id,
            CodingRoleRunStatus::Blocked,
            Some("plan_tests_timeout".to_string()),
        )
        .expect("block first run");
    let resumed = store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("resume status");
    let retry_run = store
        .supersede_latest_role_run_and_create(
            &resumed,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::RetryTestPlan,
            None,
            Some("plan_tests_timeout".to_string()),
        )
        .expect("retry run");
    assert_eq!(
        retry_run.supersedes_run_id.as_deref(),
        Some(first_run.id.as_str())
    );

    let prompts = Arc::new(Mutex::new(Vec::new()));
    let provider = TesterRetryPromptCaptureProvider {
        prompts: prompts.clone(),
    };
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    engine
        .execute_testing_with_provider(
            &resumed,
            &provider,
            &CodingExecutionContext::default(),
            &[],
            TesterAgentOptions {
                timeout: Duration::from_secs(5),
                failure_limit: 3,
            },
        )
        .await
        .expect("execute retry tester");

    let captured = prompts.lock().expect("prompts");
    let prompt = captured.first().expect("first prompt");
    assert!(prompt.contains("[previous_role_run_diagnostic]"));
    assert!(prompt.contains("reason_code: plan_tests_timeout"));
    assert!(prompt.contains("No tasks found"));
    assert!(prompt.contains("CRITICAL: Return ONLY a single JSON object"));
    let diagnostic_index = prompt
        .find("[previous_role_run_diagnostic]")
        .expect("diagnostic marker");
    let final_critical_index = prompt
        .find("CRITICAL: Return ONLY a single JSON object. Do not summarize validation.")
        .expect("final critical instruction");
    assert!(diagnostic_index < final_critical_index);
}

#[tokio::test]
async fn retry_code_review_prompt_includes_previous_role_run_diagnostic() {
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
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )
        .expect("blocked");

    let first_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0003".to_string()),
        )
        .expect("first run");
    store
        .append_role_run_event(
            &attempt,
            &first_run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Code reviewer task update",
                "status": "blocked",
                "detail": "Review context was missing"
            }),
        )
        .expect("event");
    store
        .update_role_run_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &first_run.id,
            CodingRoleRunStatus::Blocked,
            Some("code_review_blocked".to_string()),
        )
        .expect("block first run");
    let resumed = store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("resume status");
    let retry_run = store
        .supersede_latest_role_run_and_create(
            &resumed,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
            CodingRoleRunTrigger::RetryReview,
            None,
            Some("code_review_blocked".to_string()),
        )
        .expect("retry run");
    assert_eq!(
        retry_run.supersedes_run_id.as_deref(),
        Some(first_run.id.as_str())
    );

    let provider = SessionInputCapturingProvider::with_outputs(
        [r#"{"verdict":"approve","summary":"code review ok","findings":[]}"#],
        [None],
    );
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    engine
        .execute_code_review(&resumed, &provider)
        .await
        .expect("execute retry code review");

    let inputs = provider.inputs.lock().expect("inputs");
    let prompt = &inputs.first().expect("first input").prompt;
    assert!(prompt.contains("[previous_role_run_diagnostic]"));
    assert!(prompt.contains("reason_code: code_review_blocked"));
    assert!(prompt.contains("Code reviewer task update"));
    assert!(prompt.contains("Review context was missing"));
    assert!(prompt.contains("只输出 JSON"));
    assert!(!prompt.contains(&format!("role_run_id: {}", retry_run.id)));
}

#[tokio::test]
async fn coding_code_reviewer_run_uses_fresh_provider_session() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\nreviewed\n").expect("modify file");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
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
                ProviderConversationRef {
                    role: ProviderConversationRole::CodeReviewer,
                    provider: ProviderName::ClaudeCode,
                    provider_session_id: "code-reviewer-session-0".to_string(),
                    updated_at: "2026-06-01T00:02:00Z".to_string(),
                    last_node_id: Some("code-review-node-0".to_string()),
                },
            ],
        )
        .expect("persist conversations");
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
    let provider = SessionInputCapturingProvider::with_outputs(
        [r#"{"verdict":"approve","summary":"code review ok","findings":[]}"#],
        [Some("code-reviewer-session-1".to_string())],
    );

    let report = engine
        .execute_code_review(&attempt, &provider)
        .await
        .expect("code review provider run");

    assert_eq!(report.verdict, ReviewVerdict::Approve);
    let inputs = provider.inputs.lock().expect("inputs lock");
    assert_eq!(inputs.len(), 1);
    assert_eq!(
        inputs[0].permission_mode,
        ProviderPermissionMode::Supervised
    );
    assert_eq!(inputs[0].timeout_secs, 10_800);
    assert_eq!(inputs[0].resume_provider_session_id, None);
    let updated = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("updated attempt");
    assert!(updated.provider_conversations.iter().any(|conversation| {
        conversation.role == ProviderConversationRole::CodeReviewer
            && conversation.provider == ProviderName::ClaudeCode
            && conversation.provider_session_id == "code-reviewer-session-1"
    }));
}

#[tokio::test]
async fn coding_analyst_rework_uses_fresh_provider_session() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::ClaudeCode,
                reviewer: Some(ProviderName::Codex),
                review_rounds: 1,
            },
            ..create_input()
        })
        .expect("create attempt");
    let attempt = store
        .replace_attempt_provider_conversations(
            &attempt.id,
            vec![ProviderConversationRef {
                role: ProviderConversationRole::Analyst,
                provider: ProviderName::ClaudeCode,
                provider_session_id: "analyst-session-1".to_string(),
                updated_at: "2026-06-01T00:00:00Z".to_string(),
                last_node_id: Some("rework-node-1".to_string()),
            }],
        )
        .expect("persist analyst conversation");
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
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let provider = SessionInputCapturingProvider::with_outputs(
        [r#"{"verdict":"no_issue","summary":"testing ok"}"#],
        [Some("analyst-session-2".to_string())],
    );

    engine
        .execute_rework(&attempt, "testing evidence", &provider)
        .await
        .expect("analyst rework provider run");

    let inputs = provider.inputs.lock().expect("inputs lock");
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].permission_mode, ProviderPermissionMode::Auto);
    assert_eq!(inputs[0].timeout_secs, 10_800);
    assert_eq!(inputs[0].resume_provider_session_id, None);
}

#[tokio::test]
async fn coding_internal_reviewer_uses_fresh_provider_session() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\ninternal review\n").expect("modify file");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
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
            vec![ProviderConversationRef {
                role: ProviderConversationRole::InternalReviewer,
                provider: ProviderName::ClaudeCode,
                provider_session_id: "internal-reviewer-session-1".to_string(),
                updated_at: "2026-06-01T00:00:00Z".to_string(),
                last_node_id: Some("internal-review-node-1".to_string()),
            }],
        )
        .expect("persist internal reviewer conversation");
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
            CodingExecutionStage::ReviewRequest,
        )
        .expect("review request stage");
    store
        .save_review_request(&sample_review_request(&attempt.id))
        .expect("save review request");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let provider = SessionInputCapturingProvider::with_outputs(
        [
            r#"{"verdict":"approve","summary":"internal review ok","findings":[],"impact_scope":["src"],"pr_description":"实现 work item","commit_message_suggestion":"feat: implement work item"}"#,
        ],
        [Some("internal-reviewer-session-2".to_string())],
    );

    let review = engine
        .execute_internal_pr_review(&attempt, &provider)
        .await
        .expect("internal reviewer provider run");

    assert_eq!(review.verdict, ReviewVerdict::Approve);
    let inputs = provider.inputs.lock().expect("inputs lock");
    assert_eq!(inputs.len(), 1);
    assert_eq!(
        inputs[0].permission_mode,
        ProviderPermissionMode::Supervised
    );
    assert_eq!(inputs[0].timeout_secs, 10_800);
    assert_eq!(inputs[0].resume_provider_session_id, None);
}

#[tokio::test]
async fn execute_coding_includes_work_item_context_in_provider_prompt() {
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
    let captured_prompt = Arc::new(Mutex::new(None));
    let provider = PromptCapturingProvider {
        prompt: Arc::clone(&captured_prompt),
    };
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let context = CodingExecutionContext {
        work_item_markdown: Some(
            "# 爬楼梯问题 Work Item\n\n## 验证命令\n\n- `uv run python -m unittest -v tests.test_climbing_stairs`"
                .to_string(),
        ),
        verification_commands: vec![
            "uv run python -m unittest -v tests.test_climbing_stairs".to_string(),
        ],
    };

    engine
        .execute_coding(&attempt, &provider, &context)
        .await
        .expect("execute coding");

    let prompt = captured_prompt
        .lock()
        .expect("prompt lock")
        .clone()
        .expect("captured prompt");
    assert!(prompt.contains("不要只输出计划或 Story/Design/Work Item 文档"));
    assert!(prompt.contains("# 爬楼梯问题 Work Item"));
    assert!(prompt.contains("uv run python -m unittest -v tests.test_climbing_stairs"));
}

