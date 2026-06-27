#[tokio::test]
async fn execute_code_review_forwards_provider_execution_events() {
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
    let provider = EventThenCompletedProvider {
        output: r#"{"verdict":"approve","summary":"review ok","findings":[]}"#.to_string(),
    };
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    engine
        .execute_code_review(&attempt, &provider)
        .await
        .expect("execute code review");

    assert_provider_command_event(&drain_events(&mut rx));
}

#[tokio::test]
async fn execute_code_review_persists_role_run_events_while_forwarding_realtime_events() {
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
    let provider = EventThenCompletedProvider {
        output: r#"{"verdict":"approve","summary":"review ok","findings":[]}"#.to_string(),
    };
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    engine
        .execute_code_review(&attempt, &provider)
        .await
        .expect("execute code review");

    assert_provider_command_event(&drain_events(&mut rx));
    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 1);
    let events = store
        .list_role_run_events("project_0001", "issue_0001", &attempt.id, &runs[0].id)
        .expect("role run events");
    let event_types = events
        .iter()
        .map(|event| event.event_type)
        .collect::<Vec<_>>();
    assert_eq!(
        event_types,
        vec![
            CodingRoleRunEventType::ProviderPrompt,
            CodingRoleRunEventType::ProviderStart,
            CodingRoleRunEventType::ExecutionEvent,
            CodingRoleRunEventType::MessageComplete,
        ]
    );
    assert_eq!(events[2].payload["title"], "Provider command");
    assert_eq!(events[2].payload["output"], "changed files");
}

#[tokio::test]
async fn execute_code_review_persists_provider_control_and_tool_event_payloads() {
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
    let provider = ReviewControlEventProvider;
    let (tx, mut rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    engine
        .execute_code_review(&attempt, &provider)
        .await
        .expect("execute code review");

    let realtime_events = drain_events(&mut rx);
    assert!(
        realtime_events.iter().any(|event| {
            matches!(
                event,
                CodingWsOutMessage::CodingPermissionRequest { id, .. }
                    if id == "permission_review_0001"
            )
        }),
        "expected realtime permission request, got {realtime_events:?}"
    );
    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 1);
    let events = store
        .list_role_run_events("project_0001", "issue_0001", &attempt.id, &runs[0].id)
        .expect("role run events");
    let event_types = events
        .iter()
        .map(|event| event.event_type)
        .collect::<Vec<_>>();
    assert_eq!(
        event_types,
        vec![
            CodingRoleRunEventType::ProviderPrompt,
            CodingRoleRunEventType::ProviderStart,
            CodingRoleRunEventType::TextDelta,
            CodingRoleRunEventType::ExecutionEvent,
            CodingRoleRunEventType::ToolCall,
            CodingRoleRunEventType::ToolResult,
            CodingRoleRunEventType::StatusChanged,
            CodingRoleRunEventType::PermissionRequest,
            CodingRoleRunEventType::MessageComplete,
        ]
    );
    assert_eq!(events[2].payload["content"], "reviewing");
    assert_eq!(events[3].payload["event_id"], "review_command_0001");
    assert_eq!(events[3].payload["kind"], "Command");
    assert_eq!(events[3].payload["status"], "Completed");
    assert_eq!(events[3].payload["title"], "Review command");
    assert_eq!(events[3].payload["output"], "review ok");
    assert_eq!(events[4].payload["id"], "review_tool_0001");
    assert_eq!(events[4].payload["tool_name"], "run_command");
    assert_eq!(events[4].payload["input"]["command"], "cargo test --locked");
    assert_eq!(events[5].payload["tool_use_id"], "review_tool_0001");
    assert_eq!(events[5].payload["output"], "tool ok");
    assert_eq!(events[5].payload["is_error"], false);
    assert_eq!(events[6].payload["status"], "Running");
    assert_eq!(events[7].payload["id"], "permission_review_0001");
    assert_eq!(events[7].payload["tool_name"], "shell");
    assert_eq!(events[7].payload["risk_level"], "High");
    assert_eq!(
        events[8].payload["provider_session_id"],
        "review-session-0001"
    );
}

#[tokio::test]
async fn execute_code_review_records_permission_timeout_as_timeout_event() {
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
    let provider = ReviewPermissionTimeoutProvider;
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let error = engine
        .execute_code_review(&attempt, &provider)
        .await
        .expect_err("permission timeout should fail code review");

    assert!(
        error
            .to_string()
            .contains("Permission request permission_review_timeout timed out")
    );
    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 1);
    let events = store
        .list_role_run_events("project_0001", "issue_0001", &attempt.id, &runs[0].id)
        .expect("role run events");
    let timeout = events
        .iter()
        .find(|event| event.payload["permission_id"] == "permission_review_timeout")
        .expect("permission timeout event");
    assert_eq!(timeout.event_type, CodingRoleRunEventType::Timeout);
    assert_eq!(timeout.payload["reason"], "permission_timeout");
    assert_eq!(
        timeout.payload["message"],
        "Permission request permission_review_timeout timed out"
    );
}

#[tokio::test]
async fn execute_rework_forwards_provider_execution_events() {
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
    let provider = EventThenCompletedProvider {
        output: r#"{"verdict":"no_issue","summary":"testing ok"}"#.to_string(),
    };
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    engine
        .execute_rework(&attempt, "testing evidence", &provider)
        .await
        .expect("execute rework");

    assert_provider_command_event(&drain_events(&mut rx));
}

#[tokio::test]
async fn execute_internal_pr_review_forwards_provider_execution_events() {
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
        .save_review_request(&sample_review_request(&attempt.id))
        .expect("save review request");
    let provider = EventThenCompletedProvider {
        output: r#"{"verdict":"approve","summary":"internal review ok","findings":[],"impact_scope":["src"],"pr_description":"实现 work item","commit_message_suggestion":"feat: implement work item"}"#.to_string(),
    };
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    engine
        .execute_internal_pr_review(&attempt, &provider)
        .await
        .expect("execute internal review");

    assert_provider_command_event(&drain_events(&mut rx));
}

#[tokio::test]
async fn execute_testing_runs_commands_persists_report_and_emits_update() {
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
    let (tx, mut rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let specs = vec![TestCommandSpec {
        id: "unit".to_string(),
        command: vec!["sh".to_string(), "-c".to_string(), "printf ok".to_string()],
    }];

    let report = engine
        .execute_testing(&attempt, &specs)
        .await
        .expect("execute testing");

    assert_eq!(report.overall_status, TestingOverallStatus::Passed);
    assert_eq!(report.commands.len(), 1);
    assert_eq!(
        fs::read_to_string(
            store
                .attempt_test_output_root("project_0001", "issue_0001", &attempt.id)
                .join(&report.commands[0].stdout_ref)
        )
        .expect("stdout"),
        "ok"
    );
    let reports = store
        .list_testing_reports("project_0001", "issue_0001", &attempt.id)
        .expect("testing reports");
    assert_eq!(reports, vec![report.clone()]);
    let updated = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("updated attempt");
    assert_eq!(updated.stage, CodingExecutionStage::Testing);

    match rx.recv().await.expect("node created") {
        CodingWsOutMessage::CodingTimelineNodeCreated { node } => {
            assert_eq!(node.stage, CodingExecutionStage::Testing);
            assert_eq!(node.status, CodingTimelineNodeStatus::Running);
        }
        other => panic!("expected testing node created, got {other:?}"),
    }
    match rx.recv().await.expect("testing update") {
        CodingWsOutMessage::TestingReportUpdate {
            report: event_report,
        } => {
            assert_eq!(event_report.id, report.id);
            assert_eq!(event_report.overall_status, TestingOverallStatus::Passed);
        }
        other => panic!("expected testing report update, got {other:?}"),
    }
}

#[tokio::test]
async fn execute_testing_keeps_attempt_running_when_no_commands_are_available_for_analyst() {
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
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let report = engine
        .execute_testing(&attempt, &[])
        .await
        .expect("execute testing");

    assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
    let updated = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("updated attempt");
    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::Testing);
}

#[tokio::test]
async fn execute_code_review_persists_report_and_emits_review_events() {
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
    let (tx, mut rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = ReviewStreamingProvider;

    let report = engine
        .execute_code_review(&attempt, &provider)
        .await
        .expect("execute code review");

    assert_eq!(report.id, "code_review_0001");
    assert_eq!(report.attempt_id, attempt.id);
    assert_eq!(report.round, 1);
    assert_eq!(report.verdict, ReviewVerdict::Approve);
    assert_eq!(report.summary, "review ok");
    let persisted = store
        .list_code_review_reports("project_0001", "issue_0001", &attempt.id)
        .expect("code review reports");
    assert_eq!(persisted, vec![report.clone()]);
    let updated = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("updated attempt");
    assert_eq!(updated.stage, CodingExecutionStage::CodeReview);

    match rx.recv().await.expect("code review node created") {
        CodingWsOutMessage::CodingTimelineNodeCreated { node } => {
            assert_eq!(node.id, "coding_node_0001");
            assert_eq!(node.stage, CodingExecutionStage::CodeReview);
            assert_eq!(node.status, CodingTimelineNodeStatus::Running);
        }
        other => panic!("expected code review node created, got {other:?}"),
    }
    match rx.recv().await.expect("code review provider prompt") {
        CodingWsOutMessage::CodingExecutionEvent { event } => {
            assert_eq!(event.event_id, "coding_node_0001_prompt");
            assert_eq!(event.node_id.as_deref(), Some("coding_node_0001"));
            assert_eq!(event.title, "Provider Prompt");
            assert!(
                event
                    .output
                    .as_deref()
                    .is_some_and(|output| output.contains("CodeReviewer"))
            );
        }
        other => panic!("expected code review provider prompt, got {other:?}"),
    }
    assert_eq!(
        rx.recv().await.expect("code review stream chunk"),
        CodingWsOutMessage::CodingStreamChunk {
            content: "reviewing diff".to_string(),
            node_id: Some("coding_node_0001".to_string()),
        }
    );
    assert_eq!(
        rx.recv().await.expect("code review message complete"),
        CodingWsOutMessage::CodingMessageComplete {
            node_id: Some("coding_node_0001".to_string()),
        }
    );
    match rx.recv().await.expect("code review chat entry") {
        CodingWsOutMessage::CodingChatEntryCreated { entry } => {
            assert_eq!(entry.node_id.as_deref(), Some("coding_node_0001"));
            assert_eq!(entry.role, CodingAgentRole::Reviewer);
            assert_eq!(entry.entry_type, CodingEntryType::AssistantMessage);
            assert_eq!(entry.content.as_deref(), Some("review ok"));
            assert_eq!(
                entry
                    .metadata
                    .as_ref()
                    .and_then(|value| value.get("review_id"))
                    .and_then(|value| value.as_str()),
                Some("code_review_0001")
            );
        }
        other => panic!("expected code review chat entry, got {other:?}"),
    }
    match rx.recv().await.expect("code review complete") {
        CodingWsOutMessage::CodeReviewComplete {
            report: event_report,
        } => {
            assert_eq!(event_report.id, "code_review_0001");
            assert_eq!(event_report.verdict, ReviewVerdict::Approve);
        }
        other => panic!("expected code review complete, got {other:?}"),
    }
    match rx.recv().await.expect("code review node completed") {
        CodingWsOutMessage::CodingTimelineNodeUpdated {
            node_id,
            status,
            summary,
            completed_at,
        } => {
            assert_eq!(node_id, "coding_node_0001");
            assert_eq!(status, CodingTimelineNodeStatus::Completed);
            assert_eq!(summary.as_deref(), Some("code review 通过"));
            assert!(completed_at.is_some());
        }
        other => panic!("expected code review node completed, got {other:?}"),
    }
}

#[tokio::test]
async fn parses_real_provider_review_finding_aliases() {
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
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let provider = InputCapturingProvider {
        input: Arc::new(Mutex::new(None)),
        output: r#"{
            "verdict": "request_changes",
            "summary": "范围污染",
            "findings": [
                {
                    "severity": "blocking",
                    "file": "__pycache__/x.pyc",
                    "description": "不应提交运行产物",
                    "recommendation": "从提交中移除 pyc 文件",
                    "title": "运行产物进入提交"
                }
            ]
        }"#
        .to_string(),
    };

    let report = engine
        .execute_code_review(&attempt, &provider)
        .await
        .expect("execute code review");

    assert_eq!(report.verdict, ReviewVerdict::RequestChanges);
    assert_eq!(report.findings.len(), 1);
    let finding = &report.findings[0];
    assert_eq!(finding.severity, FindingSeverity::Error);
    assert_eq!(finding.file_path.as_deref(), Some("__pycache__/x.pyc"));
    assert_eq!(finding.message, "不应提交运行产物");
    assert_eq!(
        finding.required_action.as_deref(),
        Some("从提交中移除 pyc 文件")
    );
    assert_eq!(finding.source_stage, CodingExecutionStage::CodeReview);
}

#[tokio::test]
async fn review_payload_parse_failure_records_blocked_evidence_for_analyst() {
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
        output: "review output without valid json".to_string(),
    };

    let report = engine
        .execute_code_review(&attempt, &provider)
        .await
        .expect("execute code review");

    assert_eq!(report.verdict, ReviewVerdict::Blocked);
    assert!(report.summary.contains("review 输出不是有效 JSON"));
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

