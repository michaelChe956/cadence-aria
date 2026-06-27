#[tokio::test]
async fn execute_rework_consumes_next_stage_human_gate() {
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
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = AnalystStreamingProvider {
        prompt: Arc::new(Mutex::new(None)),
        output: r#"{
            "verdict":"human_required",
            "next_stage":"human_gate",
            "reason":"External browser credentials are required",
            "evidence_refs":["testing_report_0001.json"],
            "human_gate":{
                "reason_code":"external_browser_required",
                "available_actions":["provide_context","manual_continue"]
            }
        }"#
        .to_string(),
    };

    let updated = engine
        .execute_rework(&attempt, "testing blocked", &provider)
        .await
        .expect("execute rework");

    assert_eq!(updated.status, CodingAttemptStatus::Blocked);
    assert_eq!(updated.stage, CodingExecutionStage::Rework);
    let gates = store
        .list_open_blocked_gates("project_0001", "issue_0001", &attempt.id)
        .expect("open blocked gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].stage, Some(CodingExecutionStage::Rework));
    assert_eq!(gates[0].role, Some(CodingProviderRole::Analyst));
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("external_browser_required")
    );
    assert_eq!(gates[0].evidence_refs, vec!["testing_report_0001.json"]);
    assert_eq!(
        gates[0]
            .available_actions
            .iter()
            .map(|action| action.action_id.as_str())
            .collect::<Vec<_>>(),
        vec!["provide_context", "manual_continue"]
    );
    let events = drain_events(&mut rx);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            CodingWsOutMessage::CodingGateRequired { gate }
                if gate.stage.as_ref() == Some(&CodingExecutionStage::Rework)
                    && gate.role == Some(CodingProviderRole::Analyst)
        )
    }));
}

#[tokio::test]
async fn execute_rework_needs_fix_at_limit_opens_human_gate_with_warning() {
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
    store
        .increment_attempt_rework_count("project_0001", "issue_0001", &attempt.id)
        .expect("first rewrite");
    store
        .increment_attempt_rework_count("project_0001", "issue_0001", &attempt.id)
        .expect("second rewrite");
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = AnalystStreamingProvider {
        prompt: Arc::new(Mutex::new(None)),
        output: r#"{"verdict":"needs_fix","summary":"仍有失败","fix_hints":["需要人工承担风险"]}"#
            .to_string(),
    };

    let updated = engine
        .execute_rework(&attempt, "测试失败", &provider)
        .await
        .expect("execute rework");

    assert_eq!(updated.status, CodingAttemptStatus::Blocked);
    assert_eq!(updated.stage, CodingExecutionStage::Rework);
    assert_eq!(updated.rework_count, 2);
    let gates = store
        .list_open_blocked_gates("project_0001", "issue_0001", &attempt.id)
        .expect("open blocked gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].stage, Some(CodingExecutionStage::Rework));
    assert_eq!(gates[0].role, Some(CodingProviderRole::Analyst));
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("max_auto_rework_exceeded")
    );
    assert_eq!(
        gates[0]
            .available_actions
            .iter()
            .map(|action| action.action_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "continue_rework",
            "provide_context",
            "manual_continue",
            "abort",
        ]
    );
    let events = drain_events(&mut rx);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            CodingWsOutMessage::CodingChatEntryCreated { entry }
                if matches!(
                    &entry.entry_type,
                    CodingEntryType::SystemEvent { event_type, .. }
                    if event_type == "exceeded_rewrite_limit"
                )
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            CodingWsOutMessage::CodingGateRequired { gate }
                if gate.stage.as_ref() == Some(&CodingExecutionStage::Rework)
                    && gate.role == Some(CodingProviderRole::Analyst)
                    && gate.reason_code.as_deref() == Some("max_auto_rework_exceeded")
        )
    }));
}

#[tokio::test]
async fn execute_rework_needs_human_input_opens_human_gate() {
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
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = AnalystStreamingProvider {
        prompt: Arc::new(Mutex::new(None)),
        output: r#"{"verdict":"needs_human_input","questions":["n 的范围是多少？"]}"#.to_string(),
    };

    let updated = engine
        .execute_rework(&attempt, "需求不明确", &provider)
        .await
        .expect("execute rework");

    assert_eq!(updated.status, CodingAttemptStatus::Blocked);
    assert_eq!(updated.stage, CodingExecutionStage::Rework);
    let gates = store
        .list_open_blocked_gates("project_0001", "issue_0001", &attempt.id)
        .expect("open blocked gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].stage, Some(CodingExecutionStage::Rework));
    assert_eq!(gates[0].role, Some(CodingProviderRole::Analyst));
    assert_eq!(gates[0].reason_code.as_deref(), Some("analyst_human_gate"));
    let events = drain_events(&mut rx);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            CodingWsOutMessage::CodingChatEntryCreated { entry }
                if matches!(
                    &entry.entry_type,
                    CodingEntryType::AnalystVerdict {
                        verdict: AnalystVerdict::NeedsHumanInput
                    }
                ) && entry
                    .metadata
                    .as_ref()
                    .and_then(|metadata| metadata.get("questions"))
                    .and_then(|value| value.as_array())
                    .and_then(|items| items.first())
                    .and_then(|value| value.as_str())
                    == Some("n 的范围是多少？")
        )
    }));
}

#[tokio::test]
async fn execute_rework_no_issue_routes_by_previous_stage() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let testing_attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree.clone()),
            ..create_input()
        })
        .expect("create testing attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &testing_attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("testing running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &testing_attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("testing stage");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = AnalystStreamingProvider {
        prompt: Arc::new(Mutex::new(None)),
        output: r#"{"verdict":"no_issue","summary":"测试通过"}"#.to_string(),
    };

    let updated = engine
        .execute_rework(&testing_attempt, "测试通过", &provider)
        .await
        .expect("testing rework");
    assert_eq!(updated.stage, CodingExecutionStage::CodeReview);

    let review_attempt = store
        .create_attempt(CreateCodingAttemptInput {
            work_item_id: "work_item_0002".to_string(),
            branch_name: "aria/work-items/work_item_0002/attempt-1".to_string(),
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create review attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &review_attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("review running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &review_attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("review stage");
    let provider = AnalystStreamingProvider {
        prompt: Arc::new(Mutex::new(None)),
        output: r#"{"verdict":"no_issue","summary":"审查通过"}"#.to_string(),
    };

    let updated = engine
        .execute_rework(&review_attempt, "审查通过", &provider)
        .await
        .expect("review rework");
    assert_eq!(updated.stage, CodingExecutionStage::ReviewRequest);
}

#[tokio::test]
async fn execute_rework_no_issue_after_internal_review_completes_attempt() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    seed_work_item_markdown(&app_paths, "最终检查后可以完成。");
    let store = CodingAttemptStore::new(app_paths);
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
            CodingExecutionStage::InternalPrReview,
        )
        .expect("internal review stage");
    store
        .update_attempt_head_commit(
            "project_0001",
            "issue_0001",
            &attempt.id,
            Some("deadbeef".to_string()),
        )
        .expect("set head commit");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = AnalystStreamingProvider {
        prompt: Arc::new(Mutex::new(None)),
        output: r#"{"verdict":"no_issue","summary":"最终审查通过"}"#.to_string(),
    };

    let updated = engine
        .execute_rework(&attempt, "internal review ok", &provider)
        .await
        .expect("final rework");

    assert_eq!(updated.status, CodingAttemptStatus::Completed);
    assert_eq!(updated.stage, CodingExecutionStage::FinalConfirm);
    assert!(updated.completed_at.is_some());
    let nodes = store
        .get_timeline_nodes("project_0001", "issue_0001", &attempt.id)
        .expect("timeline nodes");
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0].stage, CodingExecutionStage::Rework);
    assert_eq!(nodes[0].status, CodingTimelineNodeStatus::Completed);
    assert_eq!(nodes[1].stage, CodingExecutionStage::FinalConfirm);
    assert_eq!(nodes[1].status, CodingTimelineNodeStatus::Completed);
    assert_eq!(
        nodes[1].summary.as_deref(),
        Some("Analyst 最终判定通过，attempt 已完成")
    );
}

#[tokio::test]
async fn execute_rework_invalid_json_falls_back_to_human_gate() {
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
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = AnalystStreamingProvider {
        prompt: Arc::new(Mutex::new(None)),
        output: "不是 JSON".to_string(),
    };

    let updated = engine
        .execute_rework(&attempt, "分析失败", &provider)
        .await
        .expect("execute rework");

    assert_eq!(updated.status, CodingAttemptStatus::Blocked);
    assert_eq!(updated.stage, CodingExecutionStage::Rework);
    let gates = store
        .list_open_blocked_gates("project_0001", "issue_0001", &attempt.id)
        .expect("open blocked gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].stage, Some(CodingExecutionStage::Rework));
    assert_eq!(gates[0].role, Some(CodingProviderRole::Analyst));
    assert_eq!(gates[0].reason_code.as_deref(), Some("analyst_human_gate"));
    assert_eq!(
        gates[0].raw_provider_output_ref.as_deref(),
        Some("provider-raw/rework/analyst_decision_0001.txt")
    );
    let decision = store
        .latest_analyst_decision("project_0001", "issue_0001", &attempt.id)
        .expect("latest decision")
        .expect("persisted decision");
    assert_eq!(
        decision.raw_provider_output_refs,
        vec!["provider-raw/rework/analyst_decision_0001.txt".to_string()]
    );
    let raw_path = store
        .paths()
        .root()
        .join("projects/project_0001/issues/issue_0001/coding-attempts")
        .join(&attempt.id)
        .join("provider-raw/rework/analyst_decision_0001.txt");
    assert_eq!(
        std::fs::read_to_string(raw_path).expect("raw output"),
        "不是 JSON"
    );
    let events = drain_events(&mut rx);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            CodingWsOutMessage::CodingChatEntryCreated { entry }
                if matches!(
                    &entry.entry_type,
                    CodingEntryType::AnalystVerdict {
                        verdict: AnalystVerdict::NeedsHumanInput
                    }
                ) && entry.content.as_deref() == Some("Analyst 输出不是有效 JSON，已转人工确认。")
        )
    }));
}

#[tokio::test]
async fn execute_review_request_commits_pushes_persists_request_and_emits_update() {
    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    let remote = root.path().join("remote.git");
    init_repo(&repo);
    run_git(root.path(), &["init", "--bare", remote.to_str().unwrap()]);
    run_git(
        &repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input())
        .expect("create attempt");
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let started = engine
        .start_attempt("project_0001", "issue_0001", &attempt.id)
        .await
        .expect("start attempt");
    let _stage = rx.recv().await.expect("stage event");
    let _node = rx.recv().await.expect("node event");
    let prepared = engine
        .execute_worktree_prepare(&started, &repo)
        .await
        .expect("prepare worktree");
    let _worktree_complete = rx.recv().await.expect("worktree complete");
    let worktree = prepared.worktree_path.as_ref().expect("worktree path");
    fs::write(worktree.join("src.txt"), "hello\nreview request\n").expect("modify file");

    let review_request = engine
        .execute_review_request(&prepared, "origin", "feat: implement work item")
        .await
        .expect("execute review request");

    assert_eq!(review_request.id, "review_request_0001");
    assert_eq!(review_request.attempt_id, attempt.id);
    assert_eq!(review_request.push_status, PushStatus::Pushed);
    assert_eq!(review_request.remote, "origin");
    assert_eq!(
        review_request.branch_name,
        "aria/work-items/work_item_0001/attempt-1"
    );
    assert_eq!(review_request.commit_sha.len(), 40);
    let persisted = store
        .list_review_requests("project_0001", "issue_0001", &attempt.id)
        .expect("review requests");
    assert_eq!(persisted, vec![review_request.clone()]);
    let updated = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("updated attempt");
    assert_eq!(updated.stage, CodingExecutionStage::ReviewRequest);
    assert_eq!(
        updated.head_commit.as_deref(),
        Some(review_request.commit_sha.as_str())
    );
    assert_eq!(updated.pushed_remote.as_deref(), Some("origin"));
    assert_eq!(
        updated.review_request_id.as_deref(),
        Some("review_request_0001")
    );

    match rx.recv().await.expect("review request node") {
        CodingWsOutMessage::CodingTimelineNodeCreated { node } => {
            assert_eq!(node.stage, CodingExecutionStage::ReviewRequest);
            assert_eq!(node.status, CodingTimelineNodeStatus::Running);
        }
        other => panic!("expected review request node, got {other:?}"),
    }
    match rx.recv().await.expect("review request update") {
        CodingWsOutMessage::ReviewRequestUpdate {
            review_request: event_request,
        } => {
            assert_eq!(event_request.id, review_request.id);
            assert_eq!(event_request.push_status, PushStatus::Pushed);
        }
        other => panic!("expected review request update, got {other:?}"),
    }
    match rx.recv().await.expect("review request node update") {
        CodingWsOutMessage::CodingTimelineNodeUpdated {
            status, summary, ..
        } => {
            assert_eq!(status, CodingTimelineNodeStatus::Completed);
            assert_eq!(summary.as_deref(), Some("review request 已创建"));
        }
        other => panic!("expected review request node update, got {other:?}"),
    }
}

#[tokio::test]
async fn review_request_does_not_commit_runtime_artifacts() {
    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    let remote = root.path().join("remote.git");
    init_repo(&repo);
    run_git(root.path(), &["init", "--bare", remote.to_str().unwrap()]);
    run_git(
        &repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input())
        .expect("create attempt");
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let started = engine
        .start_attempt("project_0001", "issue_0001", &attempt.id)
        .await
        .expect("start attempt");
    let _stage = rx.recv().await.expect("stage event");
    let _node = rx.recv().await.expect("node event");
    let prepared = engine
        .execute_worktree_prepare(&started, &repo)
        .await
        .expect("prepare worktree");
    let _worktree_complete = rx.recv().await.expect("worktree complete");
    let worktree = prepared.worktree_path.as_ref().expect("worktree path");
    fs::create_dir_all(worktree.join("tests/__pycache__")).expect("tests pycache");
    fs::create_dir_all(worktree.join("__pycache__")).expect("root pycache");
    fs::create_dir_all(worktree.join(".aria/coding-artifacts/test-output")).expect("artifacts");
    fs::write(
        worktree.join("climbing_stairs.py"),
        "def climb_stairs(n): return n\n",
    )
    .expect("source");
    fs::write(
        worktree.join("tests/test_climbing_stairs.py"),
        "def test_climb_stairs(): pass\n",
    )
    .expect("test");
    fs::write(
        worktree.join("__pycache__/climbing_stairs.cpython-310.pyc"),
        b"pyc",
    )
    .expect("pyc");
    fs::write(
        worktree.join("tests/__pycache__/test_climbing_stairs.cpython-310.pyc"),
        b"pyc",
    )
    .expect("test pyc");
    fs::write(
        worktree.join(".aria/coding-artifacts/test-output/planned_001.stdout.log"),
        "stdout",
    )
    .expect("stdout");

    let review_request = engine
        .execute_review_request(&prepared, "origin", "feat: implement work item")
        .await
        .expect("execute review request");

    let mut committed = git_stdout(
        worktree,
        &[
            "show",
            "--name-only",
            "--format=",
            &review_request.commit_sha,
        ],
    )
    .lines()
    .filter(|line| !line.trim().is_empty())
    .map(str::to_string)
    .collect::<Vec<_>>();
    committed.sort();
    assert_eq!(
        committed,
        vec![
            "climbing_stairs.py".to_string(),
            "tests/test_climbing_stairs.py".to_string(),
        ]
    );
}

