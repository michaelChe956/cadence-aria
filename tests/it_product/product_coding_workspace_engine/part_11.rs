#[tokio::test]
async fn execute_rework_binds_analyst_decision_chat_and_gate_to_role_run() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut input = create_input();
    input.worktree_path = Some(worktree);
    let attempt = store.create_attempt(input).expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = InputCapturingProvider {
        input: Arc::new(Mutex::new(None)),
        output: "not json".to_string(),
    };

    engine
        .execute_rework(&attempt, "testing blocked evidence", &provider)
        .await
        .expect("execute analyst");

    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].stage, CodingExecutionStage::Rework);
    assert_eq!(runs[0].role, CodingProviderRole::Analyst);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Blocked);
    assert!(
        runs[0]
            .raw_provider_output_refs
            .iter()
            .any(|value| value.contains("analyst_decision"))
    );
    assert!(
        runs[0]
            .artifact_refs
            .iter()
            .any(|value| value.contains("analyst_evidence"))
    );

    let decision = store
        .latest_analyst_decision("project_0001", "issue_0001", &attempt.id)
        .expect("latest decision")
        .expect("decision");
    assert_eq!(decision.role_run_id.as_deref(), Some(runs[0].id.as_str()));
    assert_eq!(decision.run_no, Some(1));

    let entries = store
        .list_chat_entries("project_0001", "issue_0001", &attempt.id)
        .expect("chat entries");
    assert!(entries.iter().any(|entry| {
        entry.metadata.as_ref().is_some_and(|metadata| {
            metadata.get("role_run_id").and_then(|value| value.as_str())
                == Some(runs[0].id.as_str())
                && metadata.get("run_no").and_then(|value| value.as_u64()) == Some(1)
        })
    }));
}

#[tokio::test]
async fn retry_analyst_gate_response_supersedes_latest_analyst_run() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut input = create_input();
    input.worktree_path = Some(worktree);
    let attempt = store.create_attempt(input).expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let first_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Rework,
            CodingProviderRole::Analyst,
            CodingRoleRunTrigger::Initial,
            None,
        )
        .expect("create first run");
    store
        .update_role_run_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &first_run.id,
            CodingRoleRunStatus::Blocked,
            Some("analyst_human_gate".to_string()),
        )
        .expect("block first run");
    store
        .update_role_run_refs(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &first_run.id,
            Vec::new(),
            vec!["artifacts/rework/analyst_evidence_0001.txt".to_string()],
        )
        .expect("add evidence ref");

    store
        .create_blocked_gate(CreateBlockedGateInput {
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::Rework,
            node_id: Some("coding_node_0002".to_string()),
            role: Some(CodingProviderRole::Analyst),
            title: "Analyst human gate".to_string(),
            description: "需要重跑 Analyst".to_string(),
            reason_code: Some("analyst_human_gate".to_string()),
            evidence_refs: vec!["artifacts/rework/analyst_evidence_0001.txt".to_string()],
            raw_provider_output_ref: None,
            available_actions: vec![CodingGateAction {
                action_id: "retry_analyst".to_string(),
                label: "重试 Analyst".to_string(),
                action_type: CodingGateActionType::RetryAnalyst,
            }],
        })
        .expect("create gate");

    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::WaitingForHuman,
        )
        .expect("wait for human");

    let updated = engine
        .handle_blocked_gate_response(
            "project_0001",
            "issue_0001",
            &attempt.id,
            "coding_blocked_gate_0001",
            "retry_analyst",
            None,
        )
        .await
        .expect("retry analyst");

    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::Rework);

    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 2);
    let first = runs
        .iter()
        .find(|run| run.id == first_run.id)
        .expect("first run");
    assert_eq!(first.status, CodingRoleRunStatus::Superseded);
    let second = runs
        .iter()
        .find(|run| run.id != first_run.id)
        .expect("second run");
    assert_eq!(second.status, CodingRoleRunStatus::Running);
    assert_eq!(second.trigger, CodingRoleRunTrigger::RetryAnalyst);
    assert_eq!(
        second.supersedes_run_id.as_deref(),
        Some(first_run.id.as_str())
    );
    assert_eq!(
        second.artifact_refs,
        vec!["artifacts/rework/analyst_evidence_0001.txt".to_string()]
    );
}

#[tokio::test]
async fn execute_code_review_binds_report_chat_and_status_to_role_run() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut input = create_input();
    input.worktree_path = Some(worktree);
    let attempt = store.create_attempt(input).expect("create attempt");
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
        .expect("set stage");
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = InputCapturingProvider {
        input: Arc::new(Mutex::new(None)),
        output: r#"{"verdict":"approve","summary":"review ok","findings":[]}"#.to_string(),
    };

    engine
        .execute_code_review(&attempt, &provider)
        .await
        .expect("execute code review");

    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].stage, CodingExecutionStage::CodeReview);
    assert_eq!(runs[0].role, CodingProviderRole::CodeReviewer);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Completed);
    assert_eq!(runs[0].run_no, 1);
    assert!(
        runs[0]
            .raw_provider_output_refs
            .iter()
            .any(|value| value.contains("code_review"))
    );

    let reports = store
        .list_code_review_reports("project_0001", "issue_0001", &attempt.id)
        .expect("reports");
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].role_run_id.as_deref(), Some(runs[0].id.as_str()));
    assert_eq!(reports[0].run_no, Some(1));

    let entries = store
        .list_chat_entries("project_0001", "issue_0001", &attempt.id)
        .expect("chat entries");
    assert!(entries.iter().any(|entry| {
        entry.metadata.as_ref().is_some_and(|metadata| {
            metadata.get("source").and_then(|value| value.as_str()) == Some("code_review")
                && metadata.get("role_run_id").and_then(|value| value.as_str())
                    == Some(runs[0].id.as_str())
                && metadata.get("run_no").and_then(|value| value.as_u64()) == Some(1)
        })
    }));
}

#[tokio::test]
async fn execute_internal_pr_review_binds_review_chat_and_status_to_role_run() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\ninternal reviewed\n").expect("modify file");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut input = create_input();
    input.worktree_path = Some(worktree);
    input.base_branch = "HEAD".to_string();
    let attempt = store.create_attempt(input).expect("create attempt");
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
            CodingExecutionStage::InternalPrReview,
        )
        .expect("set stage");
    store
        .save_review_request(&sample_review_request(&attempt.id))
        .expect("save review request");
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = InputCapturingProvider {
        input: Arc::new(Mutex::new(None)),
        output: r#"{"verdict":"approve","summary":"internal ok","findings":[],"impact_scope":["src/lib.rs"],"pr_description":"PR body","commit_message_suggestion":"feat: work"}"#.to_string(),
    };

    engine
        .execute_internal_pr_review(&attempt, &provider)
        .await
        .expect("execute internal review");

    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].stage, CodingExecutionStage::InternalPrReview);
    assert_eq!(runs[0].role, CodingProviderRole::InternalReviewer);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Completed);
    assert_eq!(runs[0].run_no, 1);
    assert!(
        runs[0]
            .raw_provider_output_refs
            .iter()
            .any(|value| value.contains("internal_pr_review"))
    );

    let reviews = store
        .list_internal_pr_reviews("project_0001", "issue_0001", &attempt.id)
        .expect("internal reviews");
    assert_eq!(reviews.len(), 1);
    assert_eq!(reviews[0].role_run_id.as_deref(), Some(runs[0].id.as_str()));
    assert_eq!(reviews[0].run_no, Some(1));

    let entries = store
        .list_chat_entries("project_0001", "issue_0001", &attempt.id)
        .expect("chat entries");
    assert!(entries.iter().any(|entry| {
        entry.metadata.as_ref().is_some_and(|metadata| {
            metadata.get("source").and_then(|value| value.as_str()) == Some("internal_pr_review")
                && metadata.get("role_run_id").and_then(|value| value.as_str())
                    == Some(runs[0].id.as_str())
                && metadata.get("run_no").and_then(|value| value.as_u64()) == Some(1)
                && metadata
                    .get("impact_scope")
                    .and_then(|value| value.as_array())
                    .is_some_and(|scope| scope.iter().any(|value| value == "src/lib.rs"))
        })
    }));
}
