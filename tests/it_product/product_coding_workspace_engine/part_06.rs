#[tokio::test]
async fn execute_code_review_blocked_creates_retry_gate() {
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
    crate::seed_coding_attempt_running(&store, "project_0001", "issue_0001", &attempt.id);
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
    assert_eq!(updated.status, CodingAttemptStatus::Blocked);
    assert_eq!(updated.stage, CodingExecutionStage::CodeReview);
    let gates = store
        .list_open_blocked_gates("project_0001", "issue_0001", &attempt.id)
        .expect("open blocked gates");
    assert_eq!(gates.len(), 1);
    let gate = &gates[0];
    assert_eq!(gate.stage, Some(CodingExecutionStage::CodeReview));
    assert_eq!(gate.role, Some(CodingProviderRole::CodeReviewer));
    assert_eq!(gate.reason_code.as_deref(), Some("code_review_blocked"));
    assert_eq!(
        gate.raw_provider_output_ref.as_deref(),
        Some("provider-raw/code_review/code_review_0001.txt")
    );
    assert_eq!(
        gate.available_actions
            .iter()
            .map(|action| action.action_id.as_str())
            .collect::<Vec<_>>(),
        vec!["retry_review", "send_to_coder", "abort"]
    );
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
    crate::seed_coding_attempt_running(&store, "project_0001", "issue_0001", &attempt.id);
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
    assert_eq!(updated.status, CodingAttemptStatus::Blocked);
    assert_eq!(updated.stage, CodingExecutionStage::CodeReview);
    let gates = store
        .list_open_blocked_gates("project_0001", "issue_0001", &attempt.id)
        .expect("open blocked gates");
    assert_eq!(gates.len(), 1);
    let gate = &gates[0];
    assert_eq!(gate.stage, Some(CodingExecutionStage::CodeReview));
    assert_eq!(gate.role, Some(CodingProviderRole::CodeReviewer));
    assert_eq!(
        gate.reason_code.as_deref(),
        Some("code_review_provider_interrupted")
    );
    assert_eq!(
        gate.available_actions
            .iter()
            .map(|action| action.action_id.as_str())
            .collect::<Vec<_>>(),
        vec!["retry_review"]
    );
    assert_eq!(gate.available_actions[0].label, "重试代码审查");
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
                code_reviewer: ProviderName::Codex,
                internal_reviewer: ProviderName::Fake,
                review_rounds: 1,
                permission_modes: CodingRolePermissionModes::default(),
            },
        )
        .expect("set role provider snapshot");
    crate::seed_coding_attempt_running(&store, "project_0001", "issue_0001", &attempt.id);
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
